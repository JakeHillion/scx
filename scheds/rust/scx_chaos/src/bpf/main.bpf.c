/* SPDX-License-Identifier: GPL-2.0 */
/* Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This software may be used and distributed according to the terms of the
 * GNU General Public License version 2.
 */

#define P2DQ_CREATE_STRUCT_OPS 0
#include "../../../scx_p2dq/src/bpf/main.bpf.c"

#include "intf.h"

#include <stdbool.h>

const volatile u32 random_delays_freq_frac32 = 1; /* for veristat */
const volatile u32 random_delays_min_ns = 1; /* for veristat */
const volatile u32 random_delays_max_ns = 2; /* for veristat */

struct {
	__uint(type, BPF_MAP_TYPE_TASK_STORAGE);
	__uint(map_flags, BPF_F_NO_PREALLOC);
	__type(key, int);
	__type(value, struct chaos_task_ctx);
} chaos_task_ctxs SEC(".maps");

struct chaos_task_ctx *lookup_create_chaos_task_ctx(struct task_struct *p)
{
	return bpf_task_storage_get(&chaos_task_ctxs, p, NULL, BPF_LOCAL_STORAGE_GET_F_CREATE);
}

static __always_inline enum chaos_trait_kind choose_chaos()
{
	if (bpf_get_prandom_u32() < random_delays_freq_frac32)
		return CHAOS_TRAIT_RANDOM_DELAYS;

	return CHAOS_TRAIT_NONE;
}

__weak s32 enqueue_random_delay(struct task_struct *p __arg_trusted, u64 enq_flags)
{
	const u32 min_delay = random_delays_min_ns < random_delays_max_ns ? random_delays_min_ns : random_delays_max_ns;
	const u32 max_delay = random_delays_min_ns < random_delays_max_ns ? random_delays_max_ns : random_delays_min_ns;

	// use current processor so enqueue runs here next time too
	// TODO: this assumes CPU IDs are linear, and probably needs to be mapped
	// into linear IDs with topology information passed from userspace
	u32 cpu = bpf_get_smp_processor_id();

	u64 vtime = bpf_ktime_get_ns() + min_delay;
	if (min_delay != max_delay) {
		vtime += bpf_get_prandom_u32() % (max_delay - min_delay);
	}

	scx_bpf_dsq_insert_vtime(p, DSQ_BASE | cpu, 0, vtime, enq_flags);

	return 0;
}

__weak s32 enqueue_chaotic(enum chaos_trait_kind t, struct task_struct *p __arg_trusted, u64 enq_flags)
{
	switch (t) {
	case CHAOS_TRAIT_RANDOM_DELAYS:
		return enqueue_random_delay(p, enq_flags);

	case CHAOS_TRAIT_NONE:
	case CHAOS_TRAIT_MAX:
		return false;
	}
}

s32 BPF_STRUCT_OPS_SLEEPABLE(chaos_init)
{
	struct llc_ctx *llcx;
	struct cpu_ctx *cpuc;
	int i, ret;

	bpf_for(i, 0, nr_cpus) {
		if (!(cpuc = lookup_cpu_ctx(i)) ||
		    !(llcx = lookup_llc_ctx(cpuc->llc_id)))
			return -EINVAL;

		ret = scx_bpf_create_dsq(DSQ_BASE | i, llcx->node_id);
		if (ret < 0)
			return ret;
	}

	return p2dq_init_impl();
}

void BPF_STRUCT_OPS(chaos_enqueue, struct task_struct *p __arg_trusted, u64 enq_flags)
{
	struct chaos_task_ctx *wakee_ctx;
	if (!(wakee_ctx = lookup_create_chaos_task_ctx(p)))
		goto p2dq;

	if (wakee_ctx->next_trait != CHAOS_TRAIT_NONE &&
	    enqueue_chaotic(wakee_ctx->next_trait, p, enq_flags))
		goto schedule_delayed;

p2dq:
	p2dq_enqueue_impl(p, enq_flags);

schedule_delayed:
	// TODO: schedule delayed tasks
	return;
}

void BPF_STRUCT_OPS(chaos_runnable, struct task_struct *p, u64 enq_flags)
{
	enum chaos_trait_kind t = choose_chaos();
	if (t == CHAOS_TRAIT_NONE)
		goto p2dq;

	struct chaos_task_ctx *wakee_ctx;
	if (!(wakee_ctx = lookup_create_chaos_task_ctx(p)))
		goto p2dq;

	wakee_ctx->next_trait = t;
p2dq:
	return p2dq_runnable_impl(p, enq_flags);
}

s32 BPF_STRUCT_OPS(chaos_select_cpu, struct task_struct *p, s32 prev_cpu, u64 wake_flags)
{
	struct chaos_task_ctx *wakee_ctx;
	if (!(wakee_ctx = lookup_create_chaos_task_ctx(p)))
		goto p2dq;

	// don't allow p2dq to select_cpu if we plan chaos to ensure we hit enqueue
	if (wakee_ctx->next_trait != CHAOS_TRAIT_NONE)
		return prev_cpu;

p2dq:
	return p2dq_select_cpu_impl(p, prev_cpu, wake_flags);
}

SCX_OPS_DEFINE(chaos,
	       .select_cpu		= (void *)chaos_select_cpu,
	       .enqueue			= (void *)chaos_enqueue,
	       .runnable		= (void *)chaos_runnable,
	       .init			= (void *)chaos_init,

	       .dispatch		= (void *)p2dq_dispatch,
	       .running			= (void *)p2dq_running,
	       .stopping		= (void *)p2dq_stopping,
	       .set_cpumask		= (void *)p2dq_set_cpumask,
	       .init_task		= (void *)p2dq_init_task,
	       .exit			= (void *)p2dq_exit,
	       .timeout_ms		= 30000,
	       .name			= "chaos");
