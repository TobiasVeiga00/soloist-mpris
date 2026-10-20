#!/bin/bash
# tests/installer.sh: run install.sh end to end against stand-ins.
#
# Every outside program install.sh drives (systemctl, journalctl, sudo, ufw,
# firewall-cmd, ss, ip and Soloist itself) is replaced by a small fake that
# records what it was asked, and the bridge comes from a release made here,
# so nothing on this machine is touched. Interactive runs get a terminal
# from script(1), and answers are typed once their question shows up.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
root=$(mktemp -d)
trap 'rm -rf -- "${root:?}"' EXIT

# Only what each scenario sets reaches the installer.
unset XDG_CONFIG_HOME XDG_STATE_HOME XDG_DATA_HOME DISPLAY WAYLAND_DISPLAY
unset SOLOIST_API_KEY SOLOIST_DEVICE_NAME SOLOIST_MPRIS_RELEASE_URL
export SHELL=/bin/bash INSTALL_SH="$repo/install.sh" FAKES="$root/fakes"
export PATH="$root/bin:$PATH"
mkdir -p "$root/bin" "$FAKES"

fake() { # name [dir]: write a fake program from stdin
  cat >"${2:-$root/bin}/$1"
  chmod 755 "${2:-$root/bin}/$1"
}

fake systemctl <<'EOF'
#!/bin/bash
echo "systemctl $*" >>"$FAKE/calls"
case "$*" in
  "is-active --quiet ufw") [[ ${FAKE_FIREWALL:-} == ufw ]] ;;
  "is-active --quiet firewalld") [[ ${FAKE_FIREWALL:-} == firewalld ]] ;;
esac
EOF

fake journalctl <<'EOF'
#!/bin/bash
cat "$FAKE/journal" 2>/dev/null || true
EOF

# With FAKE_SUDO_FAILS, as when nobody can type the password.
fake sudo <<'EOF'
#!/bin/bash
echo "sudo $*" >>"$FAKE/calls"
[[ -z ${FAKE_SUDO_FAILS:-} ]] || exit 1
exec "$@"
EOF

# Soloist's discovery port, on every address.
fake ss <<'EOF'
#!/bin/bash
echo 'LISTEN 0 4096 0.0.0.0:34323 0.0.0.0:* users:(("soloist",pid=4242,fd=9))'
EOF

# The default route goes through eth0, or FAKE_IFACE.
fake ip <<'EOF'
#!/bin/bash
dev=${FAKE_IFACE:-eth0}
case "$*" in
  "-4 route show default") echo "default via 192.168.7.1 dev $dev proto dhcp metric 100" ;;
  "-4 route show dev $dev proto kernel scope link") echo "192.168.7.0/24 proto kernel scope link src 192.168.7.20" ;;
esac
EOF

# ufw keeps one rule per line in $FAKE/ufw.
fake ufw <<'EOF'
#!/bin/bash
rules="$FAKE/ufw"
touch "$rules"
case $1 in
  allow)
    if grep -qxF "$*" "$rules"; then echo "Skipping adding existing rule"; else echo "$*" >>"$rules"; echo "Rule added"; fi ;;
  delete)
    shift
    grep -qxF "$*" "$rules" || { echo "Could not delete non-existent rule"; exit 0; }
    grep -vxF "$*" "$rules" >"$rules.new" || true
    mv "$rules.new" "$rules"
    echo "Rule deleted" ;;
esac
EOF

# firewalld keeps "zone<TAB>rule" lines in $FAKE/firewalld; eth0 is in "home",
# the default zone is "public".
fake firewall-cmd <<'EOF'
#!/bin/bash
rules="$FAKE/firewalld"
touch "$rules"
zone=public
for arg; do
  case $arg in
    --zone=*) zone=${arg#--zone=} ;;
    --get-zone-of-interface=eth0) echo home; exit 0 ;;
    --get-zone-of-interface=*) echo "no zone" >&2; exit 2 ;;
    --get-default-zone) echo public; exit 0 ;;
    --query-rich-rule=*) grep -qxF "$zone"$'\t'"${arg#*=}" "$rules"; exit ;;
    --add-rich-rule=*) printf '%s\t%s\n' "$zone" "${arg#*=}" >>"$rules"; exit 0 ;;
    --remove-rich-rule=*)
      grep -vxF "$zone"$'\t'"${arg#*=}" "$rules" >"$rules.new" || true
      mv "$rules.new" "$rules"
      exit 0 ;;
  esac
done
EOF

# Soloist is logged in once paired, or, with FAKE_PAIRS, as soon as the
# firewall lets the phone reach its port.
fake soloist "$FAKES" <<'EOF'
#!/bin/bash
if [[ -e $FAKE/paired ]] || { [[ -n ${FAKE_PAIRS:-} ]] && grep -qs 34323 "$FAKE/ufw" "$FAKE/firewalld"; }; then
  echo "  logged in: yes"
else
  echo "  logged in: no"
fi
EOF

# Runs the installer the way the one-line install does, piped into bash, and
# leaves its pid behind so a scenario can kill it.
fake run-install "$FAKES" <<'EOF'
#!/bin/bash
echo $$ >"$FAKE/pid"
exec bash -s -- "$@" <"$INSTALL_SH"
EOF

# A release like the ones CI publishes, with a fake updater that installs
# the fake Soloist instead of downloading the real one.
archive="soloist-mpris-$(uname -m)-linux.tar.gz"
stage="$root/stage"
mkdir -p "$stage/packaging" "$root/release" "$root/empty-release"
cp -r "$repo/packaging/systemd" "$stage/packaging/"
fake soloist-mpris "$stage" <<'EOF'
#!/bin/sh
echo "soloist-mpris (fake)"
EOF
fake soloist-update "$stage/packaging" <<'EOF'
#!/bin/bash
install -Dm 755 "$FAKES/soloist" "$HOME/.local/bin/soloist"
EOF
tar -czf "$root/release/$archive" -C "$stage" soloist-mpris packaging
(cd "$root/release" && sha256sum "$archive" >"$archive.sha256")

failures=0
# The first failure of a scenario shows what the installer printed.
fail() {
  printf '    FAIL: %s\n' "$*"
  failures=$((failures + 1))
  [[ -z $shown && -e $out ]] || return 0
  shown=1
  tr -d '\r' <"$out" | tail -n 15 | sed 's/^/      | /'
}
has() { [[ -e $1 ]] && grep -qF -- "$2" "$1"; }
expect_in() { has "$1" "$2" || fail "${3:-$1 lacks: $2}"; }
expect_not_in() { ! has "$1" "$2" || fail "${3:-$1 has: $2}"; }
expect_lines() { # file count
  local n=0
  [[ ! -e $1 ]] || n=$(grep -c '' "$1" || true)
  ((n == $2)) || fail "$1 has $n lines, expected $2"
}

scenario() { # name: a fresh home and fake state
  printf '%s\n' "$1"
  dir="$root/scenarios/${1//[^a-z0-9]/-}"
  export HOME="$dir/home" FAKE="$dir/fake"
  export SOLOIST_MPRIS_RELEASE_URL="file://$root/release"
  unset FAKE_FIREWALL FAKE_PAIRS FAKE_IFACE FAKE_SUDO_FAILS
  mkdir -p "$HOME" "$FAKE"
  out="$dir/out" status=0 shown=""
  rules_file="$HOME/.local/state/soloist-mpris/firewall-rules"
}

with_key() { # an earlier install's env file
  mkdir -p "$HOME/.config/soloist"
  printf 'SOLOIST_API_KEY=saved-key\nSOLOIST_DEVICE_NAME=Desk\n' >"$HOME/.config/soloist/env"
  chmod 600 "$HOME/.config/soloist/env"
}

# Without a controlling terminal, as under a service manager or in CI.
run_detached() {
  status=0
  setsid -w "$FAKES/run-install" "$@" </dev/null >"$out" 2>&1 || status=$?
}

# In a terminal; `answer` types replies and `finish` waits for the end.
start_tty() {
  mkfifo "$dir/in"
  script -qefc "$FAKES/run-install" /dev/null <"$dir/in" >"$out" 2>&1 &
  tty_pid=$!
  exec 3>"$dir/in"
}
wait_for() { # file text: up to 30 seconds
  local i
  for ((i = 0; i < 300; i++)); do
    has "$1" "$2" && return 0
    sleep 0.1
  done
  return 1
}
answer() { # question reply
  if ! wait_for "$out" "$1"; then
    fail "never asked: $1"
    return 0
  fi
  printf '%s\n' "$2" >&3
}
# A hidden reply waits until the terminal stops echoing, as a person would
# only type once the question is there.
answer_secret() { # question reply
  if ! wait_for "$out" "$1"; then
    fail "never asked: $1"
    return 0
  fi
  local tty i
  tty=$(ps -o tty= -p "$(cat "$FAKE/pid")" | tr -d ' ')
  for ((i = 0; i < 300; i++)); do
    stty -a -F "/dev/$tty" 2>/dev/null | grep -qw -- -echo && break
    sleep 0.1
  done
  answer "$1" "$2"
}
finish() {
  local i
  for ((i = 0; i < 300; i++)); do
    kill -0 "$tty_pid" 2>/dev/null || break
    sleep 0.1
  done
  if kill -0 "$tty_pid" 2>/dev/null; then
    fail "the installer did not finish"
    kill -9 "$(cat "$FAKE/pid")" "$tty_pid" 2>/dev/null || true
  fi
  wait "$tty_pid" || status=$?
  exec 3>&-
}

scenario "an unattended install of a paired speaker"
touch "$FAKE/paired"
SOLOIST_API_KEY="  env-secret-key " SOLOIST_DEVICE_NAME=Desk run_detached
((status == 0)) || fail "exit $status"
expect_in "$out" 'Done. "Desk" is already paired'
expect_not_in "$out" env-secret-key "the key was printed"
expect_in "$HOME/.config/soloist/env" SOLOIST_API_KEY=env-secret-key
[[ $(stat -c %a "$HOME/.config/soloist/env") == 600 ]] || fail "the env file is readable by others"
for unit in soloist.service soloist-mpris.service soloist-update.service soloist-update.timer; do
  [[ -f $HOME/.config/systemd/user/$unit ]] || fail "$unit not installed"
done
[[ -x $HOME/.local/bin/soloist-mpris ]] || fail "the bridge was not installed"
expect_in "$FAKE/calls" "systemctl --user enable --now --quiet soloist-update.timer soloist.service soloist-mpris.service"

scenario "pairing through ufw asks for the key, opens and closes the firewall"
export FAKE_FIREWALL=ufw FAKE_PAIRS=1
printf 'waiting for login\n' >"$FAKE/journal"
# The user's own rule, which must outlive the install.
printf 'allow in from 192.168.7.0/24 to any port 5353 proto udp\n' >"$FAKE/ufw"
SOLOIST_DEVICE_NAME=Desk start_tty
answer_secret "Paste the key" typed-secret-key
answer "Open it for 192.168.7.0/24 only while you pair" y
finish
((status == 0)) || fail "exit $status"
expect_in "$out" 'Paired. "Desk" is ready'
expect_not_in "$out" typed-secret-key "the key was echoed"
expect_in "$HOME/.config/soloist/env" SOLOIST_API_KEY=typed-secret-key
expect_in "$FAKE/calls" "sudo ufw allow in from 192.168.7.0/24 to any port 34323 proto tcp"
expect_in "$FAKE/ufw" "port 5353 proto udp" "the user's own rule was removed"
expect_lines "$FAKE/ufw" 1
[[ ! -s $rules_file ]] || fail "closed rules are still recorded"

scenario "firewalld rules go to the zone of the interface"
with_key
export FAKE_FIREWALL=firewalld FAKE_PAIRS=1
printf 'waiting for login\n' >"$FAKE/journal"
start_tty
answer "Open it for 192.168.7.0/24 only while you pair" y
finish
((status == 0)) || fail "exit $status"
expect_in "$out" 'Paired. "Desk" is ready'
expect_in "$FAKE/calls" "--zone=home --add-rich-rule=rule family=ipv4 source address=192.168.7.0/24 port port=5353 protocol=udp accept"
expect_lines "$FAKE/firewalld" 0

scenario "without a zone on the interface, firewalld rules go to the default zone"
with_key
export FAKE_FIREWALL=firewalld FAKE_PAIRS=1 FAKE_IFACE=wlan0
printf 'waiting for login\n' >"$FAKE/journal"
start_tty
answer "Open it for 192.168.7.0/24 only while you pair" y
finish
((status == 0)) || fail "exit $status"
expect_in "$FAKE/calls" "--zone=public --add-rich-rule=rule family=ipv4 source address=192.168.7.0/24 port port=34323 protocol=tcp accept"
expect_lines "$FAKE/firewalld" 0

scenario "rules a killed installer left open are closed by --uninstall"
with_key
export FAKE_FIREWALL=ufw
printf 'waiting for login\n' >"$FAKE/journal"
start_tty
answer "Open it for 192.168.7.0/24 only while you pair" y
wait_for "$out" "Waiting up to" || fail "pairing never started"
kill -9 "$(cat "$FAKE/pid")"
finish
expect_lines "$FAKE/ufw" 2
run_detached --uninstall
((status == 0)) || fail "uninstall exit $status"
expect_in "$out" "interrupted install"
expect_lines "$FAKE/ufw" 0
[[ ! -e $rules_file ]] || fail "the record of open rules was kept"

scenario "the next install closes rules a killed one left open"
with_key
touch "$FAKE/paired"
mkdir -p "${rules_file%/*}"
printf 'ufw\t-\tallow in from 192.168.7.0/24 to any port 34323 proto tcp\n' >"$rules_file"
printf 'allow in from 192.168.7.0/24 to any port 34323 proto tcp\n' >"$FAKE/ufw"
run_detached
((status == 0)) || fail "exit $status"
expect_in "$out" "interrupted install"
expect_lines "$FAKE/ufw" 0
[[ ! -e $rules_file ]] || fail "the record of open rules was kept"

scenario "left-over rules that cannot be removed stay recorded, and odd lines are ignored"
with_key
touch "$FAKE/paired"
mkdir -p "${rules_file%/*}"
printf 'home\trule family=ipv4 source address=192.168.7.0/24 port port=5353 protocol=udp accept\n' >"$FAKE/firewalld"
printf 'allow in from 192.168.7.0/24 to any port 34323 proto tcp\n' >"$FAKE/ufw"
# A firewalld rule, a line nobody should pass to sudo, and a ufw rule whose
# newline was lost.
printf '%s\n%s\n%s' \
  $'firewalld\thome\trule family=ipv4 source address=192.168.7.0/24 port port=5353 protocol=udp accept' \
  $'ufw\t-\tallow in from 0.0.0.0/0 to any port 22 proto tcp; touch /tmp/pwned' \
  $'ufw\t-\tallow in from 192.168.7.0/24 to any port 34323 proto tcp' >"$rules_file"
export FAKE_SUDO_FAILS=1
run_detached
((status == 0)) || fail "exit $status"
expect_in "$out" "remove it with: sudo firewall-cmd --zone=home --remove-rich-rule="
expect_in "$out" "remove it with: sudo ufw delete allow in from 192.168.7.0/24 to any port 34323 proto tcp"
expect_in "$out" "Ignoring a line this installer did not write"
expect_not_in "$FAKE/calls" "pwned" "an unexpected line reached sudo"
[[ $(grep -c 'remove it with' "$out") == 2 ]] || fail "the hints were not printed once each"
expect_lines "$rules_file" 2
unset FAKE_SUDO_FAILS
run_detached --uninstall
((status == 0)) || fail "uninstall exit $status"
expect_in "$FAKE/calls" "sudo firewall-cmd --quiet --zone=home --remove-rich-rule=rule family=ipv4"
expect_lines "$FAKE/firewalld" 0
expect_lines "$FAKE/ufw" 0
[[ ! -e $rules_file ]] || fail "the record of open rules was kept"

scenario "a release still uploading its files is reported clearly"
with_key
export SOLOIST_MPRIS_RELEASE_URL="file://$root/empty-release"
run_detached
((status == 1)) || fail "exit $status"
expect_in "$out" "could not download $archive"
expect_in "$out" "try again in a few minutes"

if ((failures)); then
  echo "$failures failed"
  exit 1
fi
echo "all passed"
