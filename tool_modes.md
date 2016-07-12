# Tool modes

Outlines functionality and relations of the `benchcmp` tool.
Notation: \[Optional\], inclusive OR, exclusive XOR

- Comparison by: *Module* (implementation variations) XOR *File* (over time)
- \[Filter OR Highlight\]: *Regressions* XOR/AND *Improvements*
- Output: *Table* XOR *Plot*
    - Table: *Comparison* of **two**
        - \[Filter by significance: *Percentage* of change\]
        - \[Detail: *Variance*\]
        - To: *Stdout* XOR *File*
    - Plot: *Comparison* of **all**
        - per *Test*: to *Directory*
        - Format: *PNG* XOR *SVG* XOR *PDF* XOR *EPS*

```
Compares Rust micro-benchmark results.

Usage:
    cargo benchcmp table [options] (<file>... | [-])
    cargo benchcmp table --by-module [options] <name> <name> (<file>... | [-])
    cargo benchcmp plot  [options] (<file>... | [-])
    cargo benchcmp -h | --help

Modes:
    table               outputs a table that compares benchmark results
    table --by-module   takes two extra arguments for the module names
                        compares benchmarks between the two modules
    plot                takes one or more files, and plots a bar-chart for
                        every bench-test it can find multiple of

General options:
    -h, --help          show this help message and exit
    --no-color          suppress coloring of improvements/regressions

Comparison options:
    --by-module         take two module names before the files and compare
                        those
    --output <file>     write to file instead of stdout
    --variance          show variance
    --threshold <n>     only show comparisons with a percentage change greater
                        than this threshold
    --regressions       show only regressions
    --improvements      show only improvements
    --strip-fst <re>    a regex to strip from first benchmarks' names
    --strip-snd <re>    a regex to strip from second benchmarks' names

Plot command options (requires gnuplot):
    --by <cmp>          plot benchmarks by file or module [default: module]
    --format <fmt>      output formats: eps, svg, pdf, png [default: png]
    --dir <dir>         directory the plots are put into [default: benchcmp]
```
