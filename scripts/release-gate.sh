#!/usr/bin/env bash
# The release A gate (PRD device-connect-iroh D-34 (1) and (2); docs/TESTING.md,
# The release A gate), run from a checkout of main on a Mac before release A:
#
#   1. the migration scenes: an SSH device switched to its machine's own Hide
#      with its terminal carried on, off when the switch is asked and back;
#      a core moved before 1.2.0 split into two paired Hides; a split that
#      stops before its commit; a machine reached over SSH paired with the
#      Hide it runs; a guest's work outliving each core;
#   2. the fault run: two Hides paired through a relay of its own, letters
#      sent from a guest's pane on its host through relay restarts, sleeps, a
#      lost network and host restarts, every one in the guest's ledger once;
#      half the minutes with the relay as their only way to each other, half
#      on their direct path beside it.
#
# Usage: scripts/release-gate.sh [fault-run minutes, 120 when unset]
# Each daemon runs under a fixture HOME in the system's temporary folder, so
# the account's own Hide, login items and devices are never touched.
set -euo pipefail

cd "$(dirname "$0")/.."

minutes=${1:-120}
case "$minutes" in '' | *[!0-9]*)
    echo "the fault run's minutes are a number, not '$minutes'" >&2
    exit 2
    ;;
esac

relay=target/iroh-relay/bin/iroh-relay
[ -x "$relay" ] || bash scripts/build-iroh-relay.sh
bash scripts/verify-cargo.sh cli

# The scenes run before the fault run and never beside it: a journey that
# waits for a guest's node to end finds it by its build's name, which the
# fault run's guest node carries too.

scenes='test(=remote_delivery::a_switch_leaves_on_its_helper_a_device_whose_hide_would_not_be_back_alone)'
scenes+=' | test(/^pairing::moved_core::/)'
scenes+=' | test(=pairing::a_split_that_stops_before_its_commit_leaves_the_node_its_core_takes)'
scenes+=' | test(=pairing::a_machine_reached_over_ssh_pairs_with_the_hide_it_already_runs)'
scenes+=' | test(/^pairing::guest_work::/)'
HIDE_TEST_HOLDER_BIN="$PWD/target/debug/hide-holder" \
    bash scripts/verify-cargo.sh nextest -p hided --test it -E "$scenes" --run-ignored all --no-fail-fast --test-threads 2

for path in relay direct; do
    HIDE_TEST_IROH_RELAY="$PWD/$relay" HIDE_TEST_FAULT_RUN_MINUTES=$(((minutes + 1) / 2)) \
        HIDE_TEST_FAULT_RUN_PATH="$path" \
        bash scripts/verify-cargo.sh nextest -p hided --test it \
        -E 'test(/^pairing::fault_run::/)' --run-ignored only --no-capture
done
