#!/usr/bin/env node

var child_process = require('child_process');
var os = require('os');

var keypress = require('keypress');

var eol = require('os').EOL;
var exec = child_process.exec;
var spawnSync = child_process.spawnSync;

if (process.argv[2] === '--help' || process.argv[2] === 'help') {
  console.log('hg-sl-up [OPTIONS] [SMARTLOG_OPTIONS] -- [UP_OR_REBASE_OPTIONS]');
  console.log('');
  console.log('select commit with keyboard from smartlog and update/rebase to it');
  console.log('');
  console.log('    Use up and down arrow keys to select previous and next commit');
  console.log('    respectively. Use left and right arrow keys to select previous and');
  console.log('    next bookmark respectively on a selected commit. Hit Enter to');
  console.log('    update to the selected commit or bookmark. Hit P to update to the');
  console.log('    parent of the selected commit instead. Hit Q, CTRL-C or Esc');
  console.log('    to exit without updating.');
  console.log('');
  console.log('    Hit R to select a commit to rebase. Move to a different commit and');
  console.log('    hit Enter to rebase onto that commit. Hit P to rebase onto its');
  console.log('    parent instead.');
  console.log('');
  console.log('    SMARTLOG_OPTIONS are options that are passed to sl smartlog.');
  console.log('    UP_OR_REBASE_OPTIONS are options that are passed to sl up or rebase.');
  console.log('');
  console.log('    For example:');
  console.log('');
  console.log('        hg-sl-up --stat -- --clean');
  console.log('');
  console.log('    shows the stats for each commit (sl smartlog --stat) and performs');
  console.log('    a clean update (sl up --clean).');
  console.log('');
  console.log('OPTIONS can be any of:');
  console.log(' --help     shows this help listing');
  process.exit(0);
}

var splitArgs = splitArgv(process.argv.slice(2));
var smartlogArgs = splitArgs[0];
var commandArgs = splitArgs[1];
var cmd = 'sl --color always smartlog ' + smartlogArgs.join(' ');

var currentCommitMarker = '@';

var output;
var commitPos;
var bookmarkIndex;
var rebasing;
var rebasingPos;

enterFullscreen();

exec(cmd, function(error, stdout, stderr) {
  output = stdout
    .replace(/\033\[(0;)?35m/g, '')
    .replace(/\r\n/g, '\n')
    .split('\n');
  commitPos = search(1, [-1, 0], /^([ \u2502\u256d\u256e\u256f\u2570\u2500~]*)@/, output);
  bookmarkIndex = -1;
  render();
});

function render() {
  var numLinesToRender = process.stdout.rows;
  var numCharsToRender = process.stdout.columns;
  var lineAfter = lineAfterCommit();
  var to = Math.max(numLinesToRender, lineAfter + 1);
  var from = to - numLinesToRender;

  renderBuffer = output
    .slice(from, to - 1)
    .map(function (line) {return line.slice(0, numCharsToRender);});

  var colors = {};
  if (bookmarkIndex !== -1) {
    colors['\033[0;33m'] = [
      [_line(commitPos) - from, bookmarkIndex + 7],
    ];
  }

  colors['\033[35m'] = colorMarkersForCommit(lineAfter, from);

  insertAll(colors, renderBuffer);
  markRebasePos(renderBuffer, from);

  process.stdout.write(
    '\033[2J' +
    '\033[0f' +
    '\033[0m' +

    renderBuffer.join('\033[0m' + eol) +
    eol
  );
}

function colorMarkersForCommit(lineAfter, lineOffset) {
  var markers = [];
  var to = lineAfter - _line(commitPos);
  var markers = [];
  for (var i = 0; i < to; i++) {
    markers.push(add(commitPos, [i - lineOffset, 2]));
  }
  return markers;
}

function markRebasePos(lines, lineOffset) {
  if (rebasing) {
    var i = _line(rebasingPos) - lineOffset;
    var line = lines[i];
    if (line) {
      var col = _col(rebasingPos);
        lines[i] =
        line.slice(0, col) + '\033[0;1m\u2190\033[0m' + line.slice(col + 1);
    }
  }
}

keypress(process.stdin);

process.stdin.on('keypress', function (ch, key) {
  if (!key) {
    return;
  }

  switch (key.name) {
    case 'up':
    case 'k':
      updateCommit(-1);
      break;
    case 'down':
    case 'j':
      updateCommit(1);
      break;
    case 'left':
      updateBookmark(-1);
      break;
    case 'right':
      updateBookmark(1);
      break;
    case 'return':
    case 'enter':
      finishCurrent();
      break;
    case 'p':
      finishParent();
      break;
    case 'r':
      rebaseFromCurrent();
      break;
  }
  if (key.ctrl && key.name == 'c'
      || key.name == 'q'
      || key.name == 'escape') {
    quit(0);
  }
});

process.stdin.setRawMode(true);
process.stdin.resume();

function updateCommit(direction) {
  commitPos =
    search(direction, commitPos, /^([ \u2502\u256d\u256e\u256f\u2570\u2500~]*)[o@]/, output) || commitPos;
  bookmarkIndex = -1;
  render();
}

function lineAfterCommit() {
  var nextCommit = search(1, commitPos, /^([ \u2502\u256d\u256e\u256f\u2570\u2500~]*)[o@]/, output);
  return nextCommit
    ? _line(nextCommit)
    : output.length;
}

function updateBookmark(direction) {
  var line = output[_line(commitPos)];
  var fromIndex = bookmarkIndex === -1 && direction === -1
    ? line.length - 1
    : bookmarkIndex + direction;
  bookmarkIndex = indexOf(direction, fromIndex, '\033[0;32m', line);
  render();
}

function finishCurrent() {
  finish(function (to) {return to;});
}

function finishParent() {
  finish(function (to) {return to + '^';});
}

function finish(toModifier) {
  if (rebasing) {
    runCommand('rebase', commandArgs.concat([
      '-s', rebasing,
      '-d', toModifier(currentTarget())
    ]));
  } else {
    runCommand('up', commandArgs.concat([toModifier(currentTarget())]));
  }
}

function rebaseFromCurrent() {
  var current = currentTarget();
  if (rebasing == current) {
    rebasing = null;
    rebasingPos = null;
  } else {
    rebasing = current;
    rebasingPos = commitPos;
  }
  render();
}

function currentTarget() {
  if (bookmarkIndex !== -1) {
    var bookmark = output[_line(commitPos)]
      .substring(bookmarkIndex)
      .match(/\033\[0;32m\s*([^\s\*]+)/)[1];

  } else {
    var commit = output[_line(commitPos)]
      .match(/[0-9a-f]{12,40}/)[0];
  }
  return bookmark || commit;
}

function runCommand(command, args) {
  leaveFullscreen();
  var result = spawnSync('sl', [command].concat(args), {stdio: 'inherit'});
  process.exit(result.status || 0);
}

function quit(code) {
  leaveFullscreen();
  process.exit(code);
}

function enterFullscreen() {
  process.stdout.write('\033[?1049h');
}

function leaveFullscreen() {
  if (process.stdin.isRaw) {
    process.stdin.setRawMode(false);
  }
  process.stdin.pause();
  process.stdout.write('\033[?1049l');
}

function splitArgv(args) {
  var sep = args.indexOf('--');
  if (sep === -1) {
    return [args, []];
  }
  return [args.slice(0, sep), args.slice(sep + 1)];
}

function insertAll(whatWhere, to) {
  return Object.keys(whatWhere).reduce(function (to, what) {
    return insert(what, whatWhere[what], to);
  }, to);
}

function insert(what, positions, inserted) {
  for (var i = 0; i < positions.length; i++) {
    var pos = positions[i];
    var oldLine = inserted[_line(pos)];
    inserted[_line(pos)] =
      oldLine.slice(0, _col(pos)) + what + oldLine.slice(_col(pos));
  }
  return inserted;
}

function search(direction, fromPos, pattern, where) {
  var len = where.length;
  var start = direction + _line(fromPos);
  for (var line = start; line < len && line >= 0; line += direction) {
    if ((column = where[line].search(pattern)) !== -1) {
      var prefix = where[line].match(pattern)[1] || '';
      return [line, column + prefix.length];
    }
  }
  return null;
}

function posOf(direction, fromPos, what, where) {
  var len = where.length;
  var fromCol = _col(fromPos);
  for (var line = _line(fromPos); line < len && line >= 0; line += direction) {
    if ((column = indexOf(direction, fromCol, what, where[line])) !== -1) {
      return [line, column];
    }
  }
  return null;
}

function indexOf(direction, fromIndex, what, where) {
  return direction === 1
    ? where.indexOf(what, fromIndex)
    : where.lastIndexOf(what, fromIndex);
}

function _col(pos) {
  return pos[1];
}

function _line(pos) {
  return pos[0];
}

function add(pos1, pos2) {
  return [_line(pos1) + _line(pos2), _col(pos1) + _col(pos2)];
}
