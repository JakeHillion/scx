# SPDX-License-Identifier: GPL-2.0
#
# Copyright (c) 2025 Meta Platforms, Inc. and affiliates.
#
# Scheduler lists for C schedulers - dynamically generated from metadata.json

# Get the directory containing this makefile
SCHEDS_C_DIR := $(dir $(lastword $(MAKEFILE_LIST)))

# Extract scheduler names from metadata.json
C_SCHEDS := $(shell jq -r 'to_entries | map(select(.value.requires_lib != true) | .key) | join(" ")' $(SCHEDS_C_DIR)metadata.json)
C_SCHEDS_LIB := $(shell jq -r 'to_entries | map(select(.value.requires_lib == true) | .key) | join(" ")' $(SCHEDS_C_DIR)metadata.json)