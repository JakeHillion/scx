/* SPDX-License-Identifier: GPL-2.0 */
/*
 * Copyright (c) 2024 Andrea Righi <arighi@nvidia.com>
 */
#include <scx/common.bpf.h>
#include "intf.h"

char _license[] SEC("license") = "GPL";

extern unsigned CONFIG_HZ __kconfig;

/*
 * Maximum task weight.
 */
#define MAX_TASK_WEIGHT		10000

/*
 * Maximum amount of voluntary context switches (this limit allows to prevent
 * spikes or abuse of the nvcsw dynamic).
 */
#define MAX_AVG_NVCSW		128

/*
 * Global DSQ used to dispatch tasks.
 */
#define SHARED_DSQ		0

/*
 * Minimum time slice that can be assigned to a task (in ns).
 */
#define SLICE_MIN	(NSEC_PER_SEC / CONFIG_HZ)

/*
 * Task time slice range.
 */
const volatile u64 slice_max = 20ULL * NSEC_PER_MSEC;
const volatile u64 slice_lag = 20ULL * NSEC_PER_MSEC;

/*
 * When enabled always dispatch all kthreads directly.
 *
 * This allows to prioritize critical kernel threads that may potentially slow
 * down the entire system if they are blocked for too long, but it may also
 * introduce interactivity issues or unfairness in scenarios with high kthread
 * activity, such as heavy I/O or network traffic.
 */
const volatile bool local_kthreads;

/*
 * Scheduling statistics.
 */
volatile u64 nr_kthread_dispatches, nr_direct_dispatches, nr_shared_dispatches;

/*
 * Exit information.
 */
UEI_DEFINE(uei);

/*
 * CPUs in the system have SMT is enabled.
 */
const volatile bool smt_enabled = true;

/*
 * Current global vruntime.
 */
static u64 vtime_now;

/*
 * Maximum possible CPU number.
 */
static u64 nr_cpu_ids;

/*
 * Per-CPU context.
 */
struct cpu_ctx {
	struct bpf_cpumask __kptr *llc_mask;
};

struct {
	__uint(type, BPF_MAP_TYPE_PERCPU_ARRAY);
	__type(key, u32);
	__type(value, struct cpu_ctx);
	__uint(max_entries, 1);
} cpu_ctx_stor SEC(".maps");

/*
 * Return a CPU context.
 */
struct cpu_ctx *try_lookup_cpu_ctx(s32 cpu)
{
	const u32 idx = 0;
	return bpf_map_lookup_percpu_elem(&cpu_ctx_stor, &idx, cpu);
}

/*
 * Per-task local storage.
 *
 * This contain all the per-task information used internally by the BPF code.
 */
struct task_ctx {
	/*
	 * Voluntary context switches metrics.
	 */
	u64 nvcsw;
	u64 nvcsw_ts;
	u64 avg_nvcsw;

	/*
	 * Task's average used time slice.
	 */
	u64 avg_runtime;
	u64 sum_runtime;
	u64 last_run_at;

	/*
	 * Task's deadline.
	 */
	u64 deadline;

	/*
	 * Task is holding a lock.
	 */
	bool lock_boost;
};

/* Map that contains task-local storage. */
struct {
	__uint(type, BPF_MAP_TYPE_TASK_STORAGE);
	__uint(map_flags, BPF_F_NO_PREALLOC);
	__type(key, int);
	__type(value, struct task_ctx);
} task_ctx_stor SEC(".maps");

/*
 * Return a local task context from a generic task.
 */
struct task_ctx *try_lookup_task_ctx(const struct task_struct *p)
{
	return bpf_task_storage_get(&task_ctx_stor,
					(struct task_struct *)p, 0, 0);
}

/*
 * Prevent excessive prioritization of tasks performing massive fsync()
 * operations on the filesystem. These tasks can degrade system responsiveness
 * by not being inherently latency-sensitive.
 */
SEC("?kprobe/vfs_fsync_range")
int kprobe_vfs_fsync_range(struct file *file, u64 start, u64 end, int datasync)
{
	struct task_struct *p = (void *)bpf_get_current_task_btf();
	struct task_ctx *tctx;

	tctx = try_lookup_task_ctx(p);
	if (tctx)
		tctx->avg_nvcsw = 0;
	return 0;
}

/*
 * Allocate/re-allocate a new cpumask.
 */
static int calloc_cpumask(struct bpf_cpumask **p_cpumask)
{
	struct bpf_cpumask *cpumask;

	cpumask = bpf_cpumask_create();
	if (!cpumask)
		return -ENOMEM;

	cpumask = bpf_kptr_xchg(p_cpumask, cpumask);
	if (cpumask)
		bpf_cpumask_release(cpumask);

	return 0;
}

/*
 * Exponential weighted moving average (EWMA).
 *
 * Copied from scx_lavd. Returns the new average as:
 *
 *	new_avg := (old_avg * .75) + (new_val * .25);
 */
static u64 calc_avg(u64 old_val, u64 new_val)
{
	return (old_val - (old_val >> 2)) + (new_val >> 2);
}

/*
 * Evaluate the EWMA limited to the range [low ... high]
 */
static u64 calc_avg_clamp(u64 old_val, u64 new_val, u64 low, u64 high)
{
	return CLAMP(calc_avg(old_val, new_val), low, high);
}

/*
 * Return true if the target task @p is a kernel thread, false instead.
 */
static inline bool is_kthread(const struct task_struct *p)
{
	return p->flags & PF_KTHREAD;
}

/*
 * Return the amount of tasks that are waiting to run.
 */
static inline u64 nr_tasks_waiting(void)
{
	return scx_bpf_dsq_nr_queued(SHARED_DSQ) + 1;
}

/*
 * Return task's weight.
 */
static u64 task_weight(const struct task_struct *p, const struct task_ctx *tctx)
{
	if (tctx->lock_boost)
		return MAX_TASK_WEIGHT;

	return p->scx.weight;
}

/*
 * Return a value proportionally scaled to the task's priority.
 */
static u64 scale_up_fair(const struct task_struct *p,
			 const struct task_ctx *tctx, u64 value)
{
	return value * task_weight(p, tctx) / 100;
}

/*
 * Return a value inversely proportional to the task's priority.
 */
static u64 scale_inverse_fair(const struct task_struct *p,
			      const struct task_ctx *tctx, u64 value)
{
	return value * 100 / task_weight(p, tctx);
}

/*
 * Return the task's allowed lag: used to determine how early its vruntime can
 * be.
 */
static u64 task_lag(const struct task_struct *p, const struct task_ctx *tctx)
{
	return scale_up_fair(p, tctx, slice_lag);
}

/*
 * ** Taken directly from fair.c in the Linux kernel **
 *
 * The "10% effect" is relative and cumulative: from _any_ nice level,
 * if you go up 1 level, it's -10% CPU usage, if you go down 1 level
 * it's +10% CPU usage. (to achieve that we use a multiplier of 1.25.
 * If a task goes up by ~10% and another task goes down by ~10% then
 * the relative distance between them is ~25%.)
 */
const int sched_prio_to_weight[40] = {
 /* -20 */     88761,     71755,     56483,     46273,     36291,
 /* -15 */     29154,     23254,     18705,     14949,     11916,
 /* -10 */      9548,      7620,      6100,      4904,      3906,
 /*  -5 */      3121,      2501,      1991,      1586,      1277,
 /*   0 */      1024,       820,       655,       526,       423,
 /*   5 */       335,       272,       215,       172,       137,
 /*  10 */       110,        87,        70,        56,        45,
 /*  15 */        36,        29,        23,        18,        15,
};

static u64 max_sched_prio(void)
{
	return ARRAY_SIZE(sched_prio_to_weight);
}

/*
 * Convert task priority to weight (following fair.c logic).
 */
static u64 sched_prio_to_latency_weight(u64 prio)
{
	u64 max_prio = max_sched_prio();

	if (prio >= max_prio) {
		scx_bpf_error("invalid priority");
		return 0;
	}

	return sched_prio_to_weight[max_prio - prio - 1];
}

/*
 * Evaluate task's deadline.
 *
 * Reuse a logic similar to scx_rusty or scx_lavd and evaluate the deadline as
 * a function of the waiting and wake-up events and the average task's runtime.
 */
static u64 task_deadline(struct task_struct *p, struct task_ctx *tctx)
{
	u64 avg_run_scaled, lat_prio, lat_weight;

	/*
	 * Evaluate the "latency priority" as a function of the average amount
	 * of context switches and the expected task runtime, using the
	 * following formula:
	 *
	 *   lat_prio = avg_nvcsw - log2(avg_run_scaled)
	 *
	 * The avg_run_scaled component is used to scale the latency priority
	 * proportionally to the task's weight and inversely proportional to
	 * its runtime, so that a task with a higher weight / shorter runtime
	 * gets a higher latency priority than a task with a lower weight /
	 * higher runtime.
	 *
	 * The log2() on the average runtime ensures that the runtime metric is
	 * more proportional and comparable to the average rate of voluntary
	 * context switches.
	 */
	avg_run_scaled = scale_inverse_fair(p, tctx, tctx->avg_runtime);
	avg_run_scaled = log2_u64(avg_run_scaled + 1);

	lat_prio = scale_up_fair(p, tctx, tctx->avg_nvcsw);
	if (lat_prio > avg_run_scaled)
		lat_prio -= avg_run_scaled;
	else
		lat_prio = 0;

	lat_prio = MIN(lat_prio, max_sched_prio() - 1);

	/*
	 * Lastly, translate the latency priority into a weight and apply it to
	 * the task's average runtime to determine the task's deadline.
	 */
	lat_weight = sched_prio_to_latency_weight(lat_prio);

	return tctx->avg_runtime * 1024 / lat_weight;
}

/*
 * Return task's evaluated deadline applied to its vruntime.
 */
static u64 task_vtime(struct task_struct *p, struct task_ctx *tctx)
{
	u64 min_vruntime = vtime_now - task_lag(p, tctx);

	/*
	 * Limit the vruntime to to avoid excessively penalizing tasks.
	 */
	if (time_before(p->scx.dsq_vtime, min_vruntime)) {
		p->scx.dsq_vtime = min_vruntime;
		tctx->deadline = p->scx.dsq_vtime + task_deadline(p, tctx);
	}

	return tctx->deadline;
}

/*
 * Evaluate task's time slice in function of the total amount of tasks that are
 * waiting to be dispatched and the task's weight.
 */
static void task_refill_slice(struct task_struct *p)
{
	struct task_ctx *tctx;
	u64 slice;

	tctx = try_lookup_task_ctx(p);
	if (!tctx)
		return;

	/*
	 * Assign a time slice proportional to the task weight and inversely
	 * proportional to the total amount of tasks that are waiting to be
	 * scheduled.
	 */
	slice = scale_up_fair(p, tctx, slice_max / nr_tasks_waiting());
	p->scx.slice = CLAMP(slice, SLICE_MIN, slice_max);
}

/*
 * Pick a target CPU for a task which is being woken up.
 *
 * If a task is dispatched here, ops.enqueue() will be skipped: task will be
 * dispatched directly to the CPU returned by this callback.
 */
s32 BPF_STRUCT_OPS(flash_select_cpu, struct task_struct *p,
		   s32 prev_cpu, u64 wake_flags)
{
	bool is_idle = false;
	s32 cpu;

	cpu = scx_bpf_select_cpu_dfl(p, prev_cpu, wake_flags, &is_idle);
	if (is_idle) {
		scx_bpf_dsq_insert(p, SCX_DSQ_LOCAL, SCX_SLICE_DFL, 0);
		__sync_fetch_and_add(&nr_direct_dispatches, 1);
	}

	return cpu;
}

/*
 * Dispatch all the other tasks that were not dispatched directly in
 * select_cpu().
 */
void BPF_STRUCT_OPS(flash_enqueue, struct task_struct *p, u64 enq_flags)
{
	struct task_ctx *tctx;

	/*
	 * Per-CPU kthreads can be critical for system responsiveness, when
	 * local_kthreads is specified they are always dispatched directly
	 * before any other task.
	 */
	if (is_kthread(p) && (local_kthreads || p->nr_cpus_allowed == 1)) {
		scx_bpf_dsq_insert(p, SCX_DSQ_LOCAL, SCX_SLICE_DFL,
				   enq_flags | SCX_ENQ_PREEMPT);
		__sync_fetch_and_add(&nr_kthread_dispatches, 1);
		return;
	}

	/*
	 * Enqueue the task to the global DSQ. The task will be dispatched on
	 * the first CPU that becomes available.
	 */
	tctx = try_lookup_task_ctx(p);
	if (!tctx)
		return;
	scx_bpf_dsq_insert_vtime(p, SHARED_DSQ, SCX_SLICE_DFL,
				 task_vtime(p, tctx), enq_flags);
	__sync_fetch_and_add(&nr_shared_dispatches, 1);

	/*
	 * Ensure the CPU currently used by the task is awake.
	 *
	 * We don't want to be overly proactive at waking idle CPUs here to
	 * increase the likelihood that CPU-intensive tasks remain on the
	 * same CPU if the system is not fully saturated (which should
	 * benefit cache-sensitive workloads), since they are re-enqueued
	 * directly via ops.enqueue() on slice exhaustion.
	 *
	 * While this may reduce work conservation for CPU-intensive tasks,
	 * it should also ensures that interactive tasks have more
	 * opportunities to find an idle CPU via ops.select_cpu(),
	 * improving their responsiveness.
	 */
	if (scx_bpf_test_and_clear_cpu_idle(cpu))
		scx_bpf_kick_cpu(cpu, SCX_KICK_IDLE);
}

void BPF_STRUCT_OPS(flash_dispatch, s32 cpu, struct task_struct *prev)
{
	/*
	 * Select a new task to run.
	 */
	if (scx_bpf_dsq_move_to_local(SHARED_DSQ))
		return;

	/*
	 * If the current task expired its time slice and no other task wants
	 * to run, simply replenish its time slice and let it run for another
	 * round on the same CPU.
	 */
	if (prev && (prev->scx.flags & SCX_TASK_QUEUED))
		task_refill_slice(prev);
}

void BPF_STRUCT_OPS(flash_running, struct task_struct *p)
{
	struct task_ctx *tctx;

	/*
	 * Refresh task's time slice immediately before it starts to run on its
	 * assigned CPU.
	 */
	task_refill_slice(p);

	tctx = try_lookup_task_ctx(p);
	if (!tctx)
		return;
	tctx->last_run_at = scx_bpf_now();

	/*
	 * Update global vruntime.
	 */
	if (time_before(vtime_now, p->scx.dsq_vtime))
		vtime_now = p->scx.dsq_vtime;
}

void BPF_STRUCT_OPS(flash_stopping, struct task_struct *p, bool runnable)
{
	u64 now = scx_bpf_now(), slice;
	s64 delta_t;
	struct task_ctx *tctx;

	tctx = try_lookup_task_ctx(p);
	if (!tctx)
		return;

	/*
	 * If the time slice is not fully depleted, it means that the task
	 * voluntarily relased the CPU, therefore update the voluntary context
	 * switch counter.
	 *
	 * NOTE: the sched_ext core implements sched_yield() by setting the
	 * time slice to 0, so we won't boost the priority of tasks that are
	 * explicitly calling sched_yield().
	 *
	 * This is actually a good thing, because we want to prioritize tasks
	 * that are releasing the CPU, because they're doing I/O, waiting for
	 * input or sending output to other tasks.
	 *
	 * Tasks that are using sched_yield() don't really need the priority
	 * boost and when they get the chance to run again they will be
	 * naturally prioritized by the vruntime-based scheduling policy.
	 */
	if (p->scx.slice > 0)
		tctx->nvcsw++;

	/*
	 * Update task's average runtime.
	 */
	slice = now - tctx->last_run_at;
	tctx->sum_runtime += slice;
	tctx->avg_runtime = calc_avg(tctx->avg_runtime, tctx->sum_runtime);

	/*
	 * Update task vruntime charging the weighted used time slice.
	 */
	p->scx.dsq_vtime += scale_inverse_fair(p, tctx, slice);
	tctx->deadline = p->scx.dsq_vtime + task_deadline(p, tctx);

	/*
	 * Refresh voluntary context switch metrics.
	 *
	 * Evaluate the average number of voluntary context switches per second
	 * using an exponentially weighted moving average, see calc_avg().
	 */
	delta_t = (s64)(now - tctx->nvcsw_ts);
	if (delta_t > NSEC_PER_SEC) {
		u64 avg_nvcsw = tctx->nvcsw * NSEC_PER_SEC / delta_t;

		tctx->nvcsw = 0;
		tctx->nvcsw_ts = now;

		/*
		 * Evaluate the latency weight of the task as its average rate
		 * of voluntary context switches (limited to to prevent
		 * excessive spikes).
		 */
		tctx->avg_nvcsw = calc_avg_clamp(tctx->avg_nvcsw, avg_nvcsw,
						 0, MAX_AVG_NVCSW);
	}
}

void BPF_STRUCT_OPS(flash_runnable, struct task_struct *p, u64 enq_flags)
{
	struct task_ctx *tctx;

	tctx = try_lookup_task_ctx(p);
	if (!tctx)
		return;
	tctx->sum_runtime = 0;
}

void BPF_STRUCT_OPS(flash_enable, struct task_struct *p)
{
	u64 now = scx_bpf_now();
	struct task_ctx *tctx;

	p->scx.dsq_vtime = vtime_now;

	tctx = try_lookup_task_ctx(p);
	if (!tctx) {
		scx_bpf_error("incorrectly initialized task: %d (%s)",
			      p->pid, p->comm);
		return;
	}
	/*
	 * Assume new tasks will use the minimum allowed time slice.
	 */
	tctx->avg_runtime = SLICE_MIN;
	tctx->nvcsw_ts = now;
	tctx->deadline = p->scx.dsq_vtime + task_deadline(p, tctx);
}

s32 BPF_STRUCT_OPS(flash_init_task, struct task_struct *p,
		   struct scx_init_task_args *args)
{
	struct task_ctx *tctx;

	tctx = bpf_task_storage_get(&task_ctx_stor, p, 0,
				    BPF_LOCAL_STORAGE_GET_F_CREATE);
	if (!tctx)
		return -ENOMEM;

	return 0;
}

static int init_cpumask(struct bpf_cpumask **cpumask)
{
	struct bpf_cpumask *mask;
	int err = 0;

	/*
	 * Do nothing if the mask is already initialized.
	 */
	mask = *cpumask;
	if (mask)
		return 0;
	/*
	 * Create the CPU mask.
	 */
	err = calloc_cpumask(cpumask);
	if (!err)
		mask = *cpumask;
	if (!mask)
		err = -ENOMEM;

	return err;
}

SEC("syscall")
int enable_sibling_cpu(struct domain_arg *input)
{
	struct cpu_ctx *cctx;
	struct bpf_cpumask *mask, **pmask;
	int err = 0;

	cctx = try_lookup_cpu_ctx(input->cpu_id);
	if (!cctx)
		return -ENOENT;

	/* Make sure the target CPU mask is initialized */
	pmask = &cctx->llc_mask;
	err = init_cpumask(pmask);
	if (err)
		return err;

	bpf_rcu_read_lock();
	mask = *pmask;
	if (mask)
		bpf_cpumask_set_cpu(input->sibling_cpu_id, mask);
	bpf_rcu_read_unlock();

	return err;
}

s32 BPF_STRUCT_OPS_SLEEPABLE(flash_init)
{
	int err;

	/* Initialize the amount of possible CPUs */
	nr_cpu_ids = scx_bpf_nr_cpu_ids();

	/*
	 * Create the shared DSQ.
	 *
	 * Allocate the new DSQ id to not clash with any valid CPU id.
	 */
	err = scx_bpf_create_dsq(SHARED_DSQ, -1);
	if (err) {
		scx_bpf_error("failed to create shared DSQ: %d", err);
		return err;
	}

	return 0;
}

void BPF_STRUCT_OPS(flash_exit, struct scx_exit_info *ei)
{
	UEI_RECORD(uei, ei);
}

SCX_OPS_DEFINE(flash_ops,
	       .select_cpu		= (void *)flash_select_cpu,
	       .enqueue			= (void *)flash_enqueue,
	       .dispatch		= (void *)flash_dispatch,
	       .running			= (void *)flash_running,
	       .stopping		= (void *)flash_stopping,
	       .runnable		= (void *)flash_runnable,
	       .enable			= (void *)flash_enable,
	       .init_task		= (void *)flash_init_task,
	       .init			= (void *)flash_init,
	       .exit			= (void *)flash_exit,
	       .flags			= SCX_OPS_ENQ_EXITING,
	       .timeout_ms		= 5000U,
	       .name			= "flash");
