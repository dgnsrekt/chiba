#!/bin/sh
# CHIBA_HERDR_PLUGIN_VERSION=2
# Installed by `chiba integration herdr`. Managed by chiba; reinstalling
# overwrites this file.
#
# herdr restores every pane as a plain shell in its saved cwd — it only
# relaunches programs for its own hardcoded agent list. chiba leaves a marker
# file while it runs; this hook reads the markers and types `chiba` back into
# the panes that had it.
#
# Runs from herdr's [[startup]] hook, so: be quiet, be idempotent, and never
# exit non-zero over one bad pane.
set -u

HERDR="${HERDR_BIN_PATH:-herdr}"
STATE="${HERDR_PLUGIN_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/herdr/plugins/dgnsrekt.chiba}"
MARKERS="$STATE/panes"

# Restored shells need a moment to print a prompt and set their title, and this
# hook fires the instant the session is back. Re-poll a few times rather than
# deciding against a pane that simply hasn't woken up yet.
ATTEMPTS=12
SLEEP=0.5

[ -d "$MARKERS" ] || exit 0
command -v jq >/dev/null 2>&1 || exit 0

attempt=0
while [ "$attempt" -lt "$ATTEMPTS" ]; do
    attempt=$((attempt + 1))
    pending=0

    panes=$("$HERDR" pane list 2>/dev/null) || exit 0

    for marker in "$MARKERS"/*.json; do
        [ -e "$marker" ] || continue

        pane_id=$(jq -r '.pane_id // empty' "$marker" 2>/dev/null)
        want_cwd=$(jq -r '.cwd // empty' "$marker" 2>/dev/null)
        [ -n "$pane_id" ] || continue

        # The pane must still exist. Pane ids survive a restart (they are
        # workspace id + public pane number, both persisted), but a pane can be
        # closed while herdr is down.
        live=$(printf '%s' "$panes" | jq -c --arg id "$pane_id" \
            '.result.panes[]? | select(.pane_id == $id)' 2>/dev/null)
        if [ -z "$live" ]; then
            rm -f "$marker"
            continue
        fi

        # Guard against the id now pointing at a different pane: the cwd has to
        # match what chiba recorded.
        have_cwd=$(printf '%s' "$live" | jq -r '.cwd // empty')
        if [ -n "$want_cwd" ] && [ "$have_cwd" != "$want_cwd" ]; then
            continue
        fi

        title=$(printf '%s' "$live" | jq -r '.terminal_title_stripped // empty')

        # Already running chiba. True after a live handoff, where the process
        # was never killed, and true for every retry pass after we send — which
        # is what makes both loops safe to repeat.
        case "$title" in
            chiba*) continue ;;
        esac

        # Only type into what looks like an idle shell prompt. herdr sets the
        # title to `user@host:path` for a bare shell; anything else is either a
        # running program we must not interrupt, or a pane that has not settled
        # yet — so count it as pending and look again next pass.
        case "$title" in
            *@*:*) ;;
            *)
                pending=1
                continue
                ;;
        esac

        "$HERDR" pane send-text "$pane_id" 'chiba
' >/dev/null 2>&1 || true
        # Give the launch a beat so the next pass sees `chiba …` as the title
        # rather than sending twice.
        pending=1
    done

    [ "$pending" -eq 0 ] && break
    sleep "$SLEEP" 2>/dev/null || break
done

exit 0
