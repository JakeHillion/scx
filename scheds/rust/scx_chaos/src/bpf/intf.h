// Copyright (c) Meta Platforms, Inc. and affiliates.

// This software may be used and distributed according to the terms of the
// GNU General Public License version 2.
#ifndef __CHAOS_INTF_H
#define __CHAOS_INTF_H

enum chaos_consts {
	DSQ_BASE_SHIFT	= 16,
	DSQ_BASE	= 1 << DSQ_BASE_SHIFT,
};

enum chaos_trait_kind {
	CHAOS_TRAIT_NONE,
	CHAOS_TRAIT_RANDOM_DELAYS,
	CHAOS_TRAIT_MAX,
};

struct chaos_task_ctx {
	// chaos_task_ctx is initialised zero'd

	enum chaos_trait_kind	next_trait;
};

#endif /* __CHAOS_INTF_H */
