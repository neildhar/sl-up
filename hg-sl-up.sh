#!/bin/bash

if [ "$1" = "--help" ] || [ "$1" = "help" ]; then
  echo "hg-sl-up [OPTIONS] [SMARTLOG_OPTIONS] -- [GOTO_OR_REBASE_OPTIONS]"
  echo ""
  echo "select commit with keyboard from smart log and update/rebase to it"
  echo ""
  echo "    Use up and down arrow keys to select previous and next commit"
  echo "    respectively. Use left and right arrow keys to select previous and"
  echo "    next bookmark respectively on a selected commit. Hit Enter to"
  echo "    update to the selected commit or bookmark. Hit P to update to the"
  echo "    parent of the selected commit instead. Hit Q, CTRL-C or Esc"
  echo "    to exit without updating."
  echo ""
  echo "    Hit R to select a commit to rebase. Move to a different commit and"
  echo "    hit Enter to rebase onto that commit. Hit P to rebase onto its"
  echo "    parent instead."
  echo ""
  echo "    SMARTLOG_OPTIONS are options that are passed to sl smartlog."
  echo "    GOTO_OR_REBASE_OPTIONS are options that are passed to sl goto or rebase."
  echo ""
  echo "    For example:"
  echo ""
  echo "        hg-sl-up --stat -- --clean"
  echo ""
  echo "    shows the stats for each commit (sl smartlog --stat) and performs"
  echo "    a clean goto (sl goto --clean)."
  echo ""
  echo "OPTIONS can be any of:"
  echo " --help     shows this help listing"
  exit
fi

# Find -- if present
sep=
args=("$@")
for ((i=0; i<${#args[@]}; i++)); do
  if [[ "${args[i]}" = "--" ]]; then
    sep="$i";
  fi
done
sep="${sep:-$#}"
to=$((sep))

# because for some reason, the range below does not compute correctly
if [ $to -gt 1 ]; then
  to=$((to+1));
fi

# split arg list
sl_args=${args[@]:0:$to}
command_args=${args[@]:($sep + 1)}

# Get path to our node module
SOURCE="${BASH_SOURCE[0]}"
while [ -h "$SOURCE" ]; do # resolve $SOURCE until the file is no longer a symlink
  DIR="$( cd -P "$( dirname "$SOURCE" )" && pwd )"
  SOURCE="$(readlink "$SOURCE")"
  [[ $SOURCE != /* ]] && SOURCE="$DIR/$SOURCE" # if $SOURCE was a relative symlink, we need to resolve it relative to the path where the symlink file was located
done
DIR="$( cd -P "$( dirname "$SOURCE" )" && pwd )"

# Actual interactive editing

tput smcup && stty -echo # enter fullscreen
node "$DIR/index.js" $sl_args

# No way to execute so node knows the window size and save output as well
# TO="$(node "$DIR/index.js" $sl_args | tee /dev/tty | tail -n1)"

UP_FILE=".____hg-sl-up-to"
REBASE_FILE=".____hg-sl-rebase-to"

# So use a tempfile

if [[ -f $UP_FILE ]]; then
  ARGS=`cat $UP_FILE`
  rm $UP_FILE
  HG_COMMAND="up"
elif [[ -f $REBASE_FILE ]]; then
  ARGS=`cat $REBASE_FILE`
  rm $REBASE_FILE
  HG_COMMAND="rebase"
else
  ARGS=""
fi

tput rmcup && stty echo && # leave fullscreen
[[ ! -z  $ARGS ]] &&
sl $HG_COMMAND ${command_args[@]} $ARGS
