use std::fmt;

pub const USAGE: &'static str = r#"
Compares Rust micro-benchmark results.

Usage:
    cargo benchcmp table [options] <file>...
    cargo benchcmp table [options] --by-module <name> <name> <file>...
    cargo benchcmp plot  [options] <file>...
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
"#;

#[derive(Debug, RustcDecodable, Clone)]
pub struct Args {
    cmd_table: bool,
    cmd_plot: bool,

    arg_file: Vec<String>,
    // NOTE: docopt cannot handle the separate name arguments apparently..
    // they're in arg_file instead
    // arg_name: Vec<String>,
    flag_by_module: bool,

    flag_no_color: bool,

    flag_output: Option<String>,
    flag_variance: bool,
    flag_threshold: Option<u8>,
    flag_regressions: bool,
    flag_improvements: bool,
    flag_strip_fst: Option<String>,
    flag_strip_snd: Option<String>,

    flag_by: CompareBy,
    flag_format: OutputFormat,
}

#[derive(Debug, RustcDecodable, PartialEq, Eq, Clone, Copy)]
pub enum CompareBy {
    File,
    Module,
}

#[derive(Debug, RustcDecodable, PartialEq, Eq, Clone, Copy)]
pub enum OutputFormat {
    Pdf,
    Eps,
    Png,
    Svg,
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        use cmd::OutputFormat::*;
        match *self {
            Pdf => f.write_str("pdf"),
            Eps => f.write_str("eps"),
            Png => f.write_str("png"),
            Svg => f.write_str("svg"),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Settings {
    pub files: Vec<String>,
    pub tool_mode: ToolMode,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum ToolMode {
    Table(TableSettings),
    Plot(PlotSettings),
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct TableSettings {
    pub compare_by: TableCompareBy,
    pub out_file: Option<String>,
    pub variance: bool,
    pub threshold: Option<u8>,
    pub show: Show,
    pub strip_fst: Option<String>,
    pub strip_snd: Option<String>,
    pub color: bool,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct PlotSettings {
    pub compare_by: CompareBy,
    pub format: OutputFormat,
    pub color: bool,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Show {
    Regressions,
    Improvements,
    Both,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum TableCompareBy {
    File,
    Module(String, String),
}

impl ToolMode {
    pub fn get_compare_by(&self) -> CompareBy {
        match *self {
            ToolMode::Table(ref settings) => {
                match settings.compare_by {
                    TableCompareBy::File => CompareBy::File,
                    TableCompareBy::Module(_, _) => CompareBy::Module,
                }
            }
            ToolMode::Plot(ref settings) => settings.compare_by,
        }
    }
}

impl Args {
    pub fn into_settings(mut self) -> Settings {
        use cmd::ToolMode::*;
        use cmd::TableCompareBy::*;
        use cmd::Show::*;

        // docopt cannot handle the separate name arguments apparently..
        // they're in arg_file instead
        let (arg_file, mut arg_name) = if self.flag_by_module {
            (self.arg_file.split_off(2), self.arg_file)
        } else {
            (self.arg_file, Vec::new())
        };

        macro_rules! extract {
            ($e:expr) => { ::std::mem::replace(&mut $e, String::default())}
        }

        Settings {
            files: arg_file,
            tool_mode: if self.cmd_plot {
                Plot(PlotSettings {
                    compare_by: self.flag_by,
                    format: self.flag_format,
                    color: !self.flag_no_color,
                })
            } else {
                Table(TableSettings {
                    compare_by: if self.flag_by_module {
                        Module(extract!(arg_name[0]), extract!(arg_name[1]))
                    } else {
                        File
                    },
                    out_file: self.flag_output,
                    variance: self.flag_variance,
                    threshold: self.flag_threshold,
                    show: if self.flag_regressions && !self.flag_improvements {
                        Regressions
                    } else if !self.flag_regressions && self.flag_improvements {
                        Improvements
                    } else {
                        Both
                    },
                    strip_fst: self.flag_strip_fst,
                    strip_snd: self.flag_strip_snd,
                    color: !self.flag_no_color,
                })
            },
        }
    }
}
