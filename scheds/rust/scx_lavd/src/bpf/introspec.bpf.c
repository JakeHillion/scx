/* SPDX-License-Identifier: GPL-2.0 */
/*
 * Copyright (c) 2023, 2024 Valve Corporation.
 * Author: Changwoo Min <changwoo@igalia.com>
 */

/*
 * To be included to the main.bpf.c
 */

/*
 * Introspection commands
 */
struct introspec intrspc;

struct {
	__uint(type, BPF_MAP_TYPE_RINGBUF);
	__uint(max_entries, 16 * 1024 /* 16 KB */);
} introspec_msg SEC(".maps");

static __always_inline
int submit_task_ctx(struct task_struct *p, struct task_ctx *taskc, u32 cpu_id)
{
	struct sys_stat *stat_cur = get_sys_stat_cur();
	struct cpu_ctx *cpuc;
	struct msg_task_ctx *m;

	cpuc = get_cpu_ctx_id(cpu_id);
	if (!cpuc)
		return -EINVAL;

	m = bpf_ringbuf_reserve(&introspec_msg, sizeof(*m), 0);
	if (!m)
		return -ENOMEM;

	m->hdr.kind = LAVD_MSG_TASKC;
	m->taskc_x.pid = p->pid;
	memcpy(m->taskc_x.comm, p->comm, TASK_COMM_LEN);
	m->taskc_x.static_prio = get_nice_prio(p);
	m->taskc_x.cpu_util = cpuc->util / 10;
	m->taskc_x.cpu_id = cpu_id;
	m->taskc_x.avg_lat_cri = stat_cur->avg_lat_cri;
	m->taskc_x.thr_perf_cri = stat_cur->thr_perf_cri;
	m->taskc_x.nr_active = stat_cur->nr_active;
	m->taskc_x.cpuperf_cur = cpuc->cpuperf_cur;

	m->taskc_x.stat[0] = is_lat_cri(taskc, stat_cur) ? 'L' : 'R';
	m->taskc_x.stat[1] = is_perf_cri(taskc, stat_cur) ? 'H' : 'I';
	m->taskc_x.stat[2] = cpuc->big_core ? 'B' : 'T';
	m->taskc_x.stat[3] = is_greedy(taskc) ? 'G' : 'E';
	m->taskc_x.stat[4] = taskc->victim_cpu >= 0 ? 'P' : 'N';
	m->taskc_x.stat[5] = '\0';

	memcpy(&m->taskc, taskc, sizeof(m->taskc));

	bpf_ringbuf_submit(m, 0);

	return 0;
}

static void proc_introspec_sched_n(struct task_struct *p,
				   struct task_ctx *taskc, u32 cpu_id)
{
	u64 cur_nr, prev_nr;
	int i;

	/* introspec_arg is the number of schedules remaining */
	cur_nr = intrspc.arg;

	/*
	 * Note that the bounded retry (@LAVD_MAX_RETRY) does *not *guarantee*
	 * to decrement introspec_arg. However, it is unlikely to happen. Even
	 * if it happens, it is nothing but a matter of delaying a message
	 * delivery. That's because other threads will try and succeed the CAS
	 * operation eventually. So this is good enough. ;-)
	 */
	for (i = 0; cur_nr > 0 && i < LAVD_MAX_RETRY; i++) {
		prev_nr = __sync_val_compare_and_swap(
				&intrspc.arg, cur_nr, cur_nr - 1);
		/* CAS success: submit a message and done */
		if (prev_nr == cur_nr) {
			submit_task_ctx(p, taskc, cpu_id);
			break;
		}
		/* CAS failure: retry */
		cur_nr = prev_nr;
	}
}

static void proc_introspec_pid(struct task_struct *p, struct task_ctx *taskc,
			       u32 cpu_id)
{
	if (p->pid == intrspc.arg)
		submit_task_ctx(p, taskc, cpu_id);
}

static void try_proc_introspec_cmd(struct task_struct *p,
				   struct task_ctx *taskc, u32 cpu_id)
{
	if (LAVD_CPU_ID_HERE == cpu_id)
		cpu_id = bpf_get_smp_processor_id();

	switch(intrspc.cmd) {
	case LAVD_CMD_SCHED_N:
		proc_introspec_sched_n(p, taskc, cpu_id);
		break;
	case LAVD_CMD_NOP:
		/* do nothing */
		break;
	default:
		scx_bpf_error("Unknown introspec command: %d", intrspc.cmd);
		break;
	}
}


