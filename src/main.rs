#![feature(inclusive_range_syntax)]
extern crate rustc_serialize;
extern crate docopt;
#[macro_use]
extern crate lazy_static;
extern crate regex;
#[macro_use]
extern crate prettytable;
extern crate gnuplot;

mod cmd;
mod benchmark;
mod utils;

use docopt::Docopt;
use prettytable::format;
use gnuplot::Figure;
use regex::Regex;

use cmd::{TableSettings, PlotSettings, CompareBy};
use benchmark::{NamedComparisons, NamedBenchmarks, Benchmark, parse_benchmarks, strip_names};

use std::io;
use std::io::prelude::*;
use std::collections::btree_map::BTreeMap;

macro_rules! err_println {
    ($fmt:expr) => (err_print!(concat!($fmt, "\n")));
    ($fmt:expr, $($arg:tt)*) => (err_print!(concat!($fmt, "\n"), $($arg)*));
}

macro_rules! err_print {
    ($($arg:tt)*) => (io::stderr().write_fmt(format_args!($($arg)*)).unwrap(););
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
    use cmd::{USAGE, Args};
    use cmd::ToolMode::*;

    let settings = {
        let args: Args = Docopt::new(USAGE)
            .and_then(|d| d.decode())
            .unwrap_or_else(|e| e.exit());
        args.into_settings()
    };

    if settings.files.is_empty() {
        err_println!("Missing argument: <file>");
        return;
    }

    // These benchmarks are maps "file -> benchmark+"
    let benchmarks = try_print_err!(read_benchmarks(settings.files));
    // These benchmarks may be maps "module -> benchmark+"
    let benchmarks = by_module_name(benchmarks, settings.tool_mode.get_compare_by());
    // These benchmark names are stripped with the given regex
    let benchmarks = try_print_err!(strip_bench_names(benchmarks, settings.strip_names));

    match settings.tool_mode {
        Table(settings) => {
            let benchmarks = try_print_err!(filter_benchmarks(benchmarks, &settings));
            // These benchmarks are maps "bench_name -> file/module -> benchmark"
            let benchmarks = by_bench_name(benchmarks);

            try_print_err!(write_pairs(benchmarks, settings));
        }
        Plot(settings) => {
            // These benchmarks are maps "bench_name -> file/module -> benchmark"
            let benchmarks = by_bench_name(benchmarks);

            try_print_err!(plot_benchmarks(benchmarks, settings));
        }
    }
}

// Reads the benchmarks from the files
fn read_benchmarks(filenames: Vec<String>) -> Result<Vec<NamedBenchmarks>, io::Error> {
    filenames.into_iter().map(parse_benchmarks).collect()
}

// Check if the benchmarks should be gathered by module instead of by file,
//  if so, do that.
fn by_module_name(nbs: Vec<NamedBenchmarks>, tool_mode: CompareBy) -> Vec<NamedBenchmarks> {
    use cmd::CompareBy::*;
    match tool_mode {
        File => nbs,
        Module => {
            let bs = nbs.into_iter().flat_map(|nb| nb.benchmarks);

            let mut map = BTreeMap::new();

            for b in bs {
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
                    NamedBenchmarks {
                        name: k.to_owned(),
                        benchmarks: v,
                    }
                })
                .collect()
        }
    }
}

fn strip_bench_names(nbs: Vec<NamedBenchmarks>,
                     option: Option<String>)
                     -> Result<Vec<NamedBenchmarks>, regex::Error> {
    Ok(match option {
        Some(s) => {
            let re = try!(Regex::new(s.as_str()));
            nbs.into_iter().map(|nb| strip_names(nb, &re)).collect()
        }
        None => nbs,
    })
}

fn by_bench_name(nbs: Vec<NamedBenchmarks>) -> Vec<NamedComparisons> {
    let mut map: BTreeMap<String, Vec<(String, Benchmark)>> = {
        nbs.iter()
            .flat_map(|nb| nb.benchmarks.iter().map(|b| b.name.clone()))
            .zip(::std::iter::once(Vec::new()).cycle())
            .collect()
    };

    for nb in nbs {
        for b in nb.benchmarks {
            map.get_mut(&b.name).unwrap().push((nb.name.clone(), b));
        }
    }

    let (singles, res) = map.into_iter()
        .map(|(k, v)| {
            NamedComparisons {
                bench_name: k,
                assocs: v,
            }
        })
        .partition(|c| c.assocs.len() < 2);

    warn_missing(singles);

    res
}

// Grabs to two to compare, filters by module name, does the regex replace
fn filter_benchmarks(mut nbs: Vec<NamedBenchmarks>,
                     settings: &TableSettings)
                     -> Result<Vec<NamedBenchmarks>, regex::Error> {
    use cmd::TableCompareBy::*;
    macro_rules! extract {
        ($e:expr) => { ::std::mem::replace(&mut $e, NamedBenchmarks::default())}
    }

    fn b_w_name(nbs: &Vec<NamedBenchmarks>, name: &String) -> NamedBenchmarks {
        nbs.iter()
            .find(|b| b.name == *name)
            .map(Clone::clone)
            .unwrap_or_else(|| NamedBenchmarks::new(name.clone()))
    }

    Ok(match settings.compare_by {
        Module(ref name_0, ref name_1) => vec![b_w_name(&nbs, name_0), b_w_name(&nbs, name_1)],
        File => vec![extract!(nbs[0]), extract!(nbs[1])],
    })
}

/// Write the pairs of benchmarks in a table, along with their comparison
fn write_pairs(pairs: Vec<NamedComparisons>, settings: TableSettings) -> Result<(), io::Error> {
    use cmd::Show::{Regressions, Improvements};
    use std::io;
    use std::fs::File;

    let mut output = prettytable::Table::new();
    output.set_format(*format::consts::FORMAT_CLEAN);

    output.add_row(row![
        b->"name",
        b->format!("{} ns/iter", pairs[0].assocs[0].0),
        b->format!("{} ns/iter", pairs[0].assocs[1].0),
        br->"diff ns/iter",
        br->"diff %"]);

    for comparison in pairs.into_iter().map(|c| c.compare(0, 1)) {
        let trunc_abs_per = (comparison.diff_ratio * 100_f64).abs().trunc() as u8;

        if settings.threshold.map_or(false, |threshold| trunc_abs_per < threshold) ||
           settings.show == Regressions && comparison.diff_ns <= 0 ||
           settings.show == Improvements && comparison.diff_ns >= 0 {
            continue;
        }

        output.add_row(comparison.to_row(settings.variance, comparison.diff_ns > 0));
    }

    match settings.out_file {
        Some(str) => {
            try!(File::create(str).and_then(|mut f| output.print(&mut f)));
        }
        None => {
            if !settings.color {
                try!(output.print(&mut io::stdout()));
            } else {
                output.printstd();
            }
        }
    }

    Ok(())
}

fn plot_benchmarks(ncs: Vec<NamedComparisons>, settings: PlotSettings) -> Result<(), io::Error> {
    use gnuplot::AxesCommon;
    use gnuplot::Tick;
    use gnuplot::TickOption;
    use gnuplot::PlotOption;
    use gnuplot::AutoOption;

    use cmd::OutputFormat::*;
    use std::path::Path;
    use std::fs::DirBuilder;

    // TODO: look up cargo environment variables to get project root
    let path = Path::new("target/benchcmp");
    match DirBuilder::new().create(path) {
        Ok(()) => {}
        Err(e) => {
            use std::io::ErrorKind::*;
            match e.kind() {
                AlreadyExists => {}
                _ => return Err(e),
            }
        }
    }

    /// Escapes strings for gnuplot. Since labels are wrapped in double quotes, we need *two*
    ///  backslashes before every underscore to make it display the underscore.
    fn escape(s: &String) -> String {
        s.replace('_', r"\\_")
    }

    println!("Writing {} images to {}", ncs.len(), path.display());

    for cs in ncs {
        let mut figure = Figure::new();

        {
            let xs = 0..cs.assocs.len();
            let x_ticks: Vec<Tick<usize>> = xs.clone()
                .map(|x| Tick::Major(x, AutoOption::Fix(escape(&cs.assocs[x].0))))
                .collect();
            let ys: Vec<usize> = cs.assocs.iter().map(|t| t.1.ns).collect();
            let y_err = cs.assocs.iter().map(|t| t.1.variance);
            let y_min = cs.assocs.iter().map(|t| t.1.ns - t.1.variance).min().unwrap() as f64 *
                        0.98;
            let y_max = cs.assocs.iter().map(|t| t.1.ns + t.1.variance).max().unwrap() as f64 *
                        1.02;
            let bench_name = escape(&cs.bench_name);
            let options = [PlotOption::Color("black"),
                           PlotOption::FillAlpha(0.6_f64),
                           PlotOption::BorderColor("#FFFFFF")];

            figure.axes2d()
                .set_title(bench_name.as_str(), &[])
                .boxes(xs.clone(), ys.clone(), &options)
                .set_x_ticks_custom(x_ticks,
                                    &[TickOption::Mirror(false), TickOption::MajorScale(0_f64)],
                                    &[])
                .set_y_label("ns/iter", &[])
                .set_y_range(AutoOption::Fix(y_min), AutoOption::Fix(y_max))
                .y_error_lines(xs,
                               ys,
                               y_err,
                               &[PlotOption::PointSize(0_f64),
                                 PlotOption::LineWidth(2_f64),
                                 PlotOption::Color("red")]);
        }

        let path = path.join(format!("{}.{}", cs.bench_name.replace("::", ".."), settings.format));

        let formatstr = match settings.format {
            Pdf => "pdfcairo",
            Eps => "epscairo",
            Png => "pngcairo",
            Svg => "svg",
        };

        figure.set_terminal(formatstr,
                            path.to_str().expect("path contains invalid unicode"));
        figure.show();
    }

    Ok(())
}

/// Print a warning message if there are benchmarks outside of the overlap
fn warn_missing(ncs: Vec<NamedComparisons>) {
    let mut map = BTreeMap::new();

    for cs in ncs {
        map.entry(cs.assocs[0].0.clone())
            .or_insert_with(Vec::new)
            .push(cs.bench_name);
    }

    for (k, v) in map {
        err_println!("WARNING: ignoring test(s) {:?} that were only found in {:?}",
                     v,
                     k);
    }
}
