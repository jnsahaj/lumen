# Benchmark

Measures how fast Lumiere starts and reacts, so a change can be compared before and
after.

```bash
python3 bench/bench.py run before                      # the lumiere on PATH
python3 bench/bench.py run after --bin target/release/lumiere
python3 bench/bench.py compare before after
```

Linux only. Needs Python 3 and Git. A run takes about two minutes.

## How it measures

The script builds four Git repositories in a temporary folder, the same ones every
time. It runs the tool in a hidden terminal of 140 by 40 and reads the time until an
expected text is on screen. Each metric is measured five times (`--runs`); the tables
show the median.

| Repository | Content |
|---|---|
| `small` | 3 changed files |
| `many` | 300 changed files |
| `large` | 1 small changed file and 1 changed file of about 5 MB |
| `ignored` | 3 changed files next to 8,000 folders that Git ignores |

## Metrics

| Metric | From | Until |
|---|---|---|
| start | launch | the first file is on screen |
| first key answered | a key pressed as soon as the first file is on screen | the second file is on screen |
| next file | the key press, one second later | the third file is on screen |
| open the large file | the key press | the large file is on screen |
| large file, whole file view | `z` | no folded line is left on screen |
| file change to screen, watch | a file is written | its new text is on screen |
| idle processor use, watch | | share of one processor over 4 seconds |
| peak memory | | highest memory in use so far |

Lumiere gathers file changes for 500 ms before it reloads, so `file change to
screen` cannot be lower than that.

## Limits

- Files are in the disk cache: a first run after a restart is not measured.
- When run inside Rex, the start includes Lumiere's search for an agent, which
  depends on the panes that are open.
- Results are saved in `bench/results/`, which Git ignores: they compare runs on one
  machine, not machines.

## Another tool

Add an entry to `TOOLS` in `bench.py`: its command, its `--watch` argument, the bytes
of its keys, and a text that is on screen only in its folded view.
