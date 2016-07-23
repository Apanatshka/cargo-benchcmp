#![feature(inclusive_range_syntax)]
extern crate rustc_serialize;
extern crate docopt;
#[macro_use]
extern crate lazy_static;
extern crate regex;
#[macro_use]
extern crate prettytable;

mod cmd;
mod benchmark;
mod utils;

use docopt::Docopt;
use prettytable::format;
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
    use cmd::OutputFormat::*;
    use std::path::Path;
    use std::fs::DirBuilder;

    let mut gnuplot_script = String::new();

    macro_rules! w {
        ($($tt:tt)*) => { gnuplot_script.push_str(format!($($tt)*).as_str()) }
    }

    // TODO: Somehow get project root? Cargo doesn't provide it in an environment variable..
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

    let term_str = match settings.format {
        Pdf => "pdfcairo",
        Eps => "epscairo",
        Png => "pngcairo",
        Svg => "svg",
    };

    err_println!("Writing {} plots to {}", ncs.len(), path.display());

    for cs in ncs {
        w!("set terminal {} noenhanced\n", term_str);

        let path = path.join(format!("{}.{}", cs.bench_name.replace("::", ".."), settings.format));

        w!("set output '{}'\n",
           path.to_str().expect("path contains invalid unicode"));

        w!("set title '{}'\n", cs.bench_name);
        w!("set ylabel 'ns/iter'\n");

        w!("set boxwidth 0.9\n");
        w!("set style data histograms\n");
        w!("set style fill solid 1.0\n");
        w!("set bars fullwidth\n");
        w!("set style fill solid border -1\n");
        w!("set style histogram errorbars gap 2 lw 1\n");

        w!("unset xtics\n");
        // round length down to even number. A little over 3 bars fit between 0 and 0.5
        let x_min = (cs.assocs.len() / 2 * 2) as f64 / 12.0;
        // round length up to even number + 4. The additional two is space for the legend.
        let x_max = ((cs.assocs.len() + 5) / 2 * 2) as f64 / 12.0;
        w!("set xrange [{:.2}:{:.2}]\n", -x_min, x_max);
        w!("set ytics border mirror norotate\n");
        let y_max = cs.assocs.iter().map( | t | t.1.ns + t.1.variance).max().unwrap() as f64 * 1.02;
        w!("set yrange [0:{:.12e}]\n", y_max);

        w!("plot ");

        {
            w!("{}", &cs.assocs.iter().map(|assoc| {
                format!("'-' binary endian=little record=1 format='%uint64' using 1:2 title '{}'", assoc.0)
            }).collect::<Vec<String>>().join(", "));
        }
        w!("\n");

        {
            use std::process::{Command, Stdio};

            let mut gnuplot_script = gnuplot_script.into_bytes();

            for assoc in &cs.assocs {
                gnuplot_script.append(&mut Vec::from(&to_bytes(assoc.1.ns as u64) as &[u8]));
                gnuplot_script.append(&mut Vec::from(&to_bytes(assoc.1.variance as u64) as &[u8]));
            }

            let gnuplot_script = gnuplot_script.as_slice();

            let process = Command::new("gnuplot").arg("-p").stdin(Stdio::piped()).spawn().ok().expect("Couldn't spawn gnuplot. Make sure it is installed and available in PATH.");
            try!(process.stdin.expect("Umm, stdin of the gnuplot process just went missing?").write(gnuplot_script).map(|_| ()));
        }

        gnuplot_script = String::new();
    }

    Ok(())
}

fn to_bytes(u: u64) -> [u8; 8] {
    unsafe {
        ::std::mem::transmute(u.to_le())
    }
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
