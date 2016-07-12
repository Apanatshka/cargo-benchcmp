#![feature(inclusive_range_syntax)]
extern crate rustc_serialize;
extern crate docopt;
#[macro_use]
extern crate lazy_static;
extern crate regex;
#[macro_use]
extern crate prettytable;
extern crate gnuplot;

mod benchmark;
mod utils;

use docopt::Docopt;
use regex::Regex;
use prettytable::format;
use gnuplot::Figure;

use benchmark::{Comparisons, Benchmarks, Benchmark, parse_benchmarks};

use std::io;
use std::io::prelude::*;
use std::fs::File;
use std::collections::btree_map::BTreeMap;
use std::fmt;

macro_rules! err_println {
    ($fmt:expr) => (err_print!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => (err_print!(concat!($fmt, "\n"), $($arg)*));
}

macro_rules! err_print {
    ($($arg:tt)*) => (io::stderr().write_fmt(format_args!($($arg)*)).unwrap(););
}

const USAGE: &'static str = r#"
Compares Rust micro-benchmark results.

Usage:
    cargo benchcmp file [options] <file> <file>
    cargo benchcmp name [options] <name> <name> <file>...
    cargo benchcmp plot [options] <bench> <file>...
    cargo benchcmp -h | --help

Commands:
    file                 takes two file and compares the common bench-tests.
    name                 takes two module names and one or more files, and
                         compares the common bench-tests of the two modules.
    plot                 takes a bench-test name and one of more files, and
                         plots a bar-chart of the bench-test in question.

General options:
    -h, --help           show this help message and exit
    --output <file>      write to this output file instead of stdout
    --variance           show variance

Comparison options:
    --threshold <n>      only show comparisons with a percentage change greater
                         than this threshold
    --show <option>      show regressions, improvements or both [default: both]
    --strip-fst <regex>  a regex to strip from first benchmarks' names
    --strip-snd <regex>  a regex to strip from second benchmarks' names
    --plot               plot the comparison instead of printing as table
                         (you can also set the --output-format when using this
                         option)

Plot command options (requires gnuplot):
    --plot-mode <mode>   plot all the benchmarks, instead of comparing two.
                         benchmarks can be grouped by file name or module
                         name [default: module]
    --output-format <format>
                         Plot output formats are: gnuplot (the commands),
                         pdf, eps, png, svg [default: png]
"#;

#[derive(Debug, RustcDecodable)]
struct Args {
    flag_output: Option<String>,
    flag_variance: bool,
    flag_threshold: Option<u8>,
    flag_show: ShowOption,
    flag_strip_fst: Option<String>,
    flag_strip_snd: Option<String>,
    flag_plot: bool,
    flag_plot_mode: ToolMode,
    flag_output_format: OutputFormat,
    cmd_plot: bool,
    arg_file: Vec<String>,
    arg_name: Option<[String; 2]>,
    arg_bench: String,
}

#[derive(Debug, RustcDecodable, PartialEq, Eq)]
enum ShowOption {
    Regressions,
    Improvements,
    Both,
}

#[derive(Debug, RustcDecodable, PartialEq, Eq)]
enum ToolMode {
    File,
    Module,
}

#[derive(Debug, RustcDecodable, PartialEq, Eq)]
enum OutputFormat {
    Gnuplot,
    Pdf,
    Eps,
    Png,
    Svg,
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        use OutputFormat::*;
        match *self {
            Gnuplot => f.write_str("gnuplot"),
            Pdf => f.write_str("pdf"),
            Eps => f.write_str("eps"),
            Png => f.write_str("png"),
            Svg => f.write_str("svg"),
        }
    }
}

struct Settings {
    out_mode: OutputMode,
    tool_mode: ToolMode,
    variance: bool,
    filenames: Vec<String>,
    output_file: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum OutputMode {
    Table(ComparisonDetails),
    Plot(PlotSubject, OutputFormat),
}

#[derive(Debug, PartialEq, Eq)]
struct ComparisonDetails {
    names: Option<[String; 2]>,
    threshold: Option<u8>,
    show: ShowOption,
    strip_fst: Option<String>,
    strip_snd: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum PlotSubject {
    Benchmark(String),
    Comparison(ComparisonDetails),
}

impl Args {
    fn into_settings(self) -> Settings {
        use OutputMode::*;
        use PlotSubject::*;

        Settings {
            output_file: if self.cmd_plot && self.flag_output_format != OutputFormat::Gnuplot {
                let name = Some(format!("benchcmp_{}.{}", self.arg_bench, self.flag_output_format));
                if self.flag_output.is_none() {
                    name
                } else {
                    let s = self.flag_output.unwrap();
                    if s.is_empty() {
                        name
                    } else {
                        Some(s)
                    }
                }
            } else {
                self.flag_output
            },
            variance: self.flag_variance,
            filenames: self.arg_file.clone(),
            tool_mode: if self.cmd_plot {
                self.flag_plot_mode
            } else {
                match self.arg_name {
                    Some(_) => ToolMode::Module,
                    None => ToolMode::File,
                }
            },
            out_mode: if self.cmd_plot {
                Plot(Benchmark(self.arg_bench), self.flag_output_format)
            } else {
                let details = ComparisonDetails {
                    names: self.arg_name,
                    threshold: self.flag_threshold,
                    show: self.flag_show,
                    strip_fst: self.flag_strip_fst,
                    strip_snd: self.flag_strip_snd,
                };
                if !self.flag_plot {
                    Table(details)
                } else {
                    Plot(Comparison(details), self.flag_output_format)
                }
            },
        }
    }
}

macro_rules! try_print_err {
    ($e:expr) => {match $e {
        Ok(res) => res,
        Err(e) => {
            err_println!("{}", e);
            return;
        },
    }}
}

fn main() {
    use OutputMode::*;
    use PlotSubject::*;

    let settings = {
        let args: Args = Docopt::new(USAGE)
            .and_then(|d| d.decode())
            .unwrap_or_else(|e| e.exit());
        args.into_settings()
    };

    // These benchmarks are maps "file -> benchmark+"
    let benchmarks = try_print_err!(read_benchmarks(settings.filenames));
    // These benchmarks may be maps "module -> benchmark+"
    let benchmarks = by_module_name(benchmarks, settings.tool_mode);

    match settings.out_mode {
        Plot(Comparison(ref details), _) |
        Table(ref details) => {
            let benchmarks = try_print_err!(filter_benchmarks(benchmarks, details));
            // These benchmarks are maps "bench_name -> file/module -> benchmark"
            let benchmarks = by_bench_name(benchmarks);

            match settings.out_mode {
                Plot(_, format) => {
                    //                    let figure = plot_benchmarks(benchmarks);
                    //
                    //                    write_plot(figure, format, settings.output_file);
                }
                Table(_) => {
                    try_print_err!(write_pairs(benchmarks,
                                               settings.output_file,
                                               settings.variance,
                                               details));
                }
            }
        }
        Plot(Benchmark(bench), format) => {
            // These benchmarks are maps "bench_name -> file/module -> benchmark"
            let benchmarks = by_bench_name(benchmarks);

            let benchmark = benchmarks.into_iter()
                .find(|c| c.bench_name == bench)
                .expect(format!("ERROR: Cannot find benchmark {}", bench).as_str());

            let figure = plot_benchmark(benchmark);

            write_plot(figure, format, settings.output_file);
        }
    }
}

// Reads the benchmarks from the files
fn read_benchmarks(filenames: Vec<String>) -> Result<Vec<Benchmarks>, io::Error> {
    filenames.into_iter().map(parse_benchmarks).collect()
}

// Check if the benchmarks should be gathered by module instead of by file,
//  if so, do that.
fn by_module_name(benchmarks: Vec<Benchmarks>, tool_mode: ToolMode) -> Vec<Benchmarks> {
    match tool_mode {
        ToolMode::File => benchmarks,
        ToolMode::Module => {
            let benchmarks = benchmarks.into_iter().flat_map(|b| b.benchmarks);

            let mut map = BTreeMap::new();

            for b in benchmarks {
                let mut split = b.name.splitn(2, "::");

                let module = String::from(split.next().unwrap());

                let (module, test) = if let Some(test) = split.next() {
                    (module, String::from(test))
                } else {
                    (String::from(""), module)
                };
                let b = Benchmark { name: test, ..b };
                map.entry(module).or_insert_with(Vec::new).push(b);
            }

            map.into_iter()
                .map(|(k, v)| {
                    Benchmarks {
                        name: k.to_owned(),
                        benchmarks: v,
                    }
                })
                .collect()
        }
    }
}

fn by_bench_name(benchmarks: Vec<Benchmarks>) -> Vec<Comparisons> {
    let mut map: BTreeMap<String, Vec<(String, Benchmark)>> = {
        benchmarks.iter()
            .flat_map(|b| b.benchmarks.iter().map(|b| b.name.clone()))
            .zip(::std::iter::once(Vec::new()).cycle())
            .collect()
    };

    for benches in benchmarks {
        for bench in benches.benchmarks {
            map.get_mut(&bench.name).unwrap().push((benches.name.clone(), bench));
        }
    }

    map.into_iter()
        .map(|(k, v)| {
            Comparisons {
                bench_name: k,
                assocs: v,
            }
        })
        .collect()
}

// Grabs to two to compare, filters by module name, does the regex replace
fn filter_benchmarks(benchmarks: Vec<Benchmarks>,
                     details: &ComparisonDetails)
                     -> Result<Vec<Benchmarks>, regex::Error> {
    if let Some(ref names) = details.names {
        Ok(vec![try!(strip_names(benchmarks.iter()
                                     .find(|b| b.name == names[0])
                                     .map(Clone::clone)
                                     .unwrap_or_else(|| Benchmarks::new(names[0].clone())),
                                 &details.strip_fst)),
                try!(strip_names(benchmarks.into_iter()
                                     .find(|b| b.name == names[1])
                                     .unwrap_or_else(|| Benchmarks::new(names[1].clone())),
                                 &details.strip_snd))])
    } else {
        Ok(vec![try!(strip_names(benchmarks[0].clone(), &details.strip_fst)),
                try!(strip_names(benchmarks[1].clone(), &details.strip_snd))])
    }


}

/// Write the pairs of benchmarks in a table, along with their comparison
fn write_pairs(pairs: Vec<Comparisons>,
               file: Option<String>,
               variance: bool,
               details: &ComparisonDetails)
               -> Result<(), io::Error> {
    use ShowOption::{Regressions, Improvements};

    let (singles, pairs) = pairs.into_iter().partition(|c| c.assocs.len() < 2);

    warn_missing(singles);

    let mut output = prettytable::Table::new();
    output.set_format(*format::consts::FORMAT_CLEAN);

    output.add_row(row![
        d->"name",
        format!("{} ns/iter", pairs[0].assocs[0].0),
        format!("{} ns/iter", pairs[0].assocs[1].0),
        r->"diff ns/iter",
        r->"diff %"]);

    for comparison in pairs.into_iter().map(|c| c.compare(0, 1)) {
        let trunc_abs_per = (comparison.diff_ratio * 100f64).abs().trunc() as u8;

        if details.threshold.map_or(false, |threshold| trunc_abs_per < threshold) ||
           details.show == Regressions && comparison.diff_ns <= 0 ||
           details.show == Improvements && comparison.diff_ns >= 0 {
            continue;
        }

        output.add_row(comparison.to_row(variance));
    }

    match file {
        Some(str) => {
            try!(File::create(str).and_then(|mut f| output.print(&mut f)));
        }
        None => {
            output.printstd();
        }
    }

    Ok(())
}

fn plot_benchmark(comparisons: Comparisons) -> Figure {
    use gnuplot::AxesCommon;
    use gnuplot::Tick;
    use gnuplot::TickOption;
    use gnuplot::PlotOption;
    use gnuplot::AutoOption;

    /// Escapes strings for gnuplot. Since labels are wrapped in double quotes, we need *two*
    ///  backslashes before every underscore to make it display the underscore.
    fn escape(s: &String) -> String {
        s.replace('_', r"\\_")
    }

    let mut figure = Figure::new();

    {
        let log_base = 10f64;
        let xs = 0..comparisons.assocs.len();
        let x_ticks: Vec<Tick<usize>> = xs.clone().map(|x| Tick::Major(x, AutoOption::Fix(escape(&comparisons.assocs[x].0)))).collect();
        let ys: Vec<usize> = comparisons.assocs.iter().map(|t| t.1.ns).collect();
        let y_err = comparisons.assocs.iter().map(|t| t.1.variance);
        let y_min = comparisons.assocs.iter().map(|t| t.1.ns - t.1.variance).min().unwrap() as f64 * 0.98;
        let y_max = comparisons.assocs.iter().map(|t| t.1.ns + t.1.variance).max().unwrap() as f64 * 1.02;
        let bench_name = escape(&comparisons.bench_name);
        let options = [PlotOption::Color("#555555"), PlotOption::BorderColor("#FFFFFF")];

        figure.axes2d()
            .set_title(bench_name.as_str(), &[])
            .boxes(xs.clone(), ys.clone(), &options)
            .set_x_ticks_custom(x_ticks, &[TickOption::Mirror(false)], &[])
            .set_y_label("ns/iter (log 10 scale)", &[])
            .set_y_log(Some(log_base))
            .set_y_ticks(Some((AutoOption::Auto, 10)), &[], &[])
            .set_y_range(AutoOption::Fix(y_min), AutoOption::Fix(y_max))
            .y_error_lines(xs, ys, y_err, &[PlotOption::PointSize(0f64), PlotOption::LineWidth(2f64), PlotOption::Color("red")]);
    }

    figure
}

fn write_plot(mut figure: Figure, format: OutputFormat, file: Option<String>) {
    use OutputFormat::*;

    let filestr = file.as_ref().map(String::as_str).unwrap_or("");
    let formatstr = match format {
        Gnuplot => {
            match file {
                Some(ref s) => {
                    figure.echo_to_file(s.as_str());
                }
                None => {
                    figure.echo(&mut io::stdout());
                }
            }
            return;
        }
        Pdf => "pdfcairo",
        Eps => "epscairo",
        Png => "pngcairo",
        Svg => "svg",
    };

    figure.set_terminal(formatstr, filestr);
    figure.show();
}

/// Filter the names in every benchmark, based on the regex string
fn strip_names(mut benches: Benchmarks,
               strip: &Option<String>)
               -> Result<Benchmarks, regex::Error> {
    match *strip {
        None => Ok(benches),
        Some(ref s) => {
            let re = try!(Regex::new(s.as_str()));
            benches.benchmarks = benches.benchmarks
                .into_iter()
                .map(|mut b| {
                    b.filter_name(&re);
                    b
                })
                .collect();
            Ok(benches)
        }
    }
}

/// Print a warning message if there are benchmarks outside of the overlap
fn warn_missing(v: Vec<Comparisons>) {
    let mut map = BTreeMap::new();

    for comparisons in v {
        map.entry(comparisons.assocs[0].0.clone())
            .or_insert_with(Vec::new)
            .push(comparisons.bench_name);
    }

    for (k, v) in map {
        err_println!("WARNING: found test(s) {:?} only in {}", v, k);
    }
}
