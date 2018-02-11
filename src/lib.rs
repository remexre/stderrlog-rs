// Copyright 2016 Doug Goldstein
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! A simple logger to provide symantics similar to what is expected
//! of most UNIX utilities by logging to stderr and the higher the
//! verbosity the higher the log level. Additionally it supports the
//! ability to provide timestamps at different granularities.
//!
//! ## Simple Use Case
//!
//! ```rust
//! #[macro_use]
//! extern crate log;
//! extern crate stderrlog;
//!
//! fn main() {
//!     stderrlog::new().module(module_path!()).init().unwrap();
//!
//!     info!("starting up");
//!
//!     // ...
//! }
//! ```
//!
//! ## docopt Example
//!
//! ```rust
//! extern crate docopt;
//! #[macro_use]
//! extern crate log;
//! extern crate rustc_serialize;
//! extern crate stderrlog;
//!
//! use docopt::Docopt;
//!
//! const USAGE: &'static str = "
//! Usage: program [-q] [-v...]
//! ";
//!
//! #[derive(Debug, RustcDecodable)]
//! struct Args {
//!     flag_q: bool,
//!     flag_v: usize,
//! }
//!
//! fn main() {
//!     let args: Args = Docopt::new(USAGE)
//!                             .and_then(|d| d.decode())
//!                             .unwrap_or_else(|e| e.exit());
//!
//!     stderrlog::new()
//!             .module(module_path!())
//!             .quiet(args.flag_q)
//!             .timestamp(stderrlog::Timestamp::Second)
//!             .verbosity(args.flag_v)
//!             .init()
//!             .unwrap();
//!     trace!("trace message");
//!     debug!("debug message");
//!     info!("info message");
//!     warn!("warn message");
//!     error!("error message");
//!
//!     // ...
//! }
//! ```
//!
//! # clap Example
//!
//! ```
//! #[macro_use]
//! extern crate clap;
//! #[macro_use]
//! extern crate log;
//! extern crate stderrlog;
//!
//! use clap::{Arg, App};
//!
//! fn main() {
//!     let m = App::new("stderrlog example")
//!         .version(crate_version!())
//!         .arg(Arg::with_name("verbosity")
//!              .short("v")
//!              .multiple(true)
//!              .help("Increase message verbosity"))
//!         .arg(Arg::with_name("quiet")
//!              .short("q")
//!              .help("Silence all output"))
//!         .arg(Arg::with_name("timestamp")
//!              .short("t")
//!              .help("prepend log lines with a timestamp")
//!              .takes_value(true)
//!              .possible_values(&["none", "sec", "ms", "ns"]))
//!         .get_matches();
//!
//!     let verbose = m.occurrences_of("verbosity") as usize;
//!     let quiet = m.is_present("quiet");
//!     let ts = match m.value_of("timestamp") {
//!         Some("ns") => stderrlog::Timestamp::Nanosecond,
//!         Some("ms") => stderrlog::Timestamp::Microsecond,
//!         Some("sec") => stderrlog::Timestamp::Second,
//!         Some("none") | None => stderrlog::Timestamp::Off,
//!         Some(_) => clap::Error {
//!             message: "invalid value for 'timestamp'".into(),
//!             kind: clap::ErrorKind::InvalidValue,
//!             info: None,
//!         }.exit(),
//!     };
//!
//!     stderrlog::new()
//!         .module(module_path!())
//!         .quiet(quiet)
//!         .verbosity(verbose)
//!         .timestamp(ts)
//!         .init()
//!         .unwrap();
//!     trace!("trace message");
//!     debug!("debug message");
//!     info!("info message");
//!     warn!("warn message");
//!     error!("error message");
//! }
//! ```

extern crate chrono;
extern crate log;
extern crate termcolor;
extern crate thread_local;

use chrono::Local;
use log::{Level, LevelFilter, Log, Metadata, Record};
use std::cell::RefCell;
use std::fmt;
use std::io::{self, Write};
use termcolor::{Color, ColorSpec, StandardStream, WriteColor};
pub use termcolor::ColorChoice;
use thread_local::CachedThreadLocal;

/// State of the timestampping in the logger.
#[derive(Clone, Copy, Debug)]
pub enum Timestamp {
    /// Disable timestamping of log messages
    Off,
    /// Timestamp with second granularity
    Second,
    /// Timestamp with microsecond granularity
    Microsecond,
    /// Timestamp with nanosecond granularity
    Nanosecond,
}

/// Data specific to this logger
pub struct StdErrLog {
    verbosity: LevelFilter,
    quiet: bool,
    timestamp: Timestamp,
    modules: Vec<String>,
    writer: CachedThreadLocal<RefCell<io::LineWriter<StandardStream>>>,
    color_choice: ColorChoice,
}

impl fmt::Debug for StdErrLog {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("StdErrLog")
            .field("verbosity", &self.verbosity)
            .field("quiet", &self.quiet)
            .field("timestamp", &self.timestamp)
            .field("modules", &self.modules)
            .field("writer", &"stderr")
            .field("color_choice", &self.color_choice)
            .finish()
    }
}

impl Clone for StdErrLog {
    fn clone(&self) -> StdErrLog {
        StdErrLog {
            modules: self.modules.clone(),
            writer: CachedThreadLocal::new(),
            ..*self
        }
    }
}

impl Log for StdErrLog {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.log_level_filter()
    }

    fn log(&self, record: &Record) {
        // if logging isn't enabled for this level do a quick out
        if !self.enabled(record.metadata()) {
            return;
        }

        // this logger only logs the requested modules unless the
        // vector of modules is empty
        // modules will have module::file in the module_path
        let should_log = match record.module_path() {
            Some(module) => self.includes_module(module),
            None => true,
        };
        if should_log {
            let writer = self.writer.get_or(|| {
                Box::new(RefCell::new(io::LineWriter::new(
                    StandardStream::stderr(self.color_choice),
                )))
            });
            let mut writer = writer.borrow_mut();
            let color = match record.metadata().level() {
                Level::Error => Color::Red,
                Level::Warn => Color::Magenta,
                Level::Info => Color::Yellow,
                Level::Debug => Color::Cyan,
                Level::Trace => Color::Blue,
            };
            {
                writer
                    .get_mut()
                    .set_color(ColorSpec::new().set_fg(Some(color)))
                    .expect("failed to set color");
            }
            match self.timestamp {
                Timestamp::Second => {
                    let fmt = "%Y-%m-%dT%H:%M:%S%:z";
                    let _ = write!(writer, "{} - ", Local::now().format(fmt));
                }
                Timestamp::Microsecond => {
                    let fmt = "%Y-%m-%dT%H:%M:%S%.6f%:z";
                    let _ = write!(writer, "{} - ", Local::now().format(fmt));
                }
                Timestamp::Nanosecond => {
                    let fmt = "%Y-%m-%dT%H:%M:%S%.9f%:z";
                    let _ = write!(writer, "{} - ", Local::now().format(fmt));
                }
                Timestamp::Off => {}
            }
            let _ = writeln!(writer, "{} - {}", record.level(), record.args());
            {
                writer.get_mut().reset().expect("failed to reset the color");
            }
        }
    }

    fn flush(&self) {
        let writer = self.writer.get_or(|| {
            Box::new(RefCell::new(io::LineWriter::new(
                StandardStream::stderr(self.color_choice),
            )))
        });
        let mut writer = writer.borrow_mut();
        writer.flush().ok();
    }
}

impl StdErrLog {
    /// creates a new stderr logger
    pub fn new() -> StdErrLog {
        StdErrLog {
            verbosity: LevelFilter::Error,
            quiet: false,
            timestamp: Timestamp::Off,
            modules: Vec::new(),
            writer: CachedThreadLocal::new(),
            color_choice: ColorChoice::Auto,
        }
    }

    /// Sets the verbosity level of messages that will be displayed
    pub fn verbosity(&mut self, verbosity: usize) -> &mut StdErrLog {
        let log_lvl = match verbosity {
            0 => LevelFilter::Error,
            1 => LevelFilter::Warn,
            2 => LevelFilter::Info,
            3 => LevelFilter::Debug,
            _ => LevelFilter::Trace,
        };

        self.verbosity = log_lvl;
        self
    }

    /// silence all output, no matter the value of verbosity
    pub fn quiet(&mut self, quiet: bool) -> &mut StdErrLog {
        self.quiet = quiet;
        self
    }

    /// Enables or disables the use of timestamps in log messages
    pub fn timestamp(&mut self, timestamp: Timestamp) -> &mut StdErrLog {
        self.timestamp = timestamp;
        self
    }

    /// Enables or disables the use of color in log messages
    pub fn color(&mut self, choice: ColorChoice) -> &mut StdErrLog {
        self.color_choice = choice;
        self
    }

    /// specify a module to allow to log to stderr
    pub fn module<T: Into<String>>(&mut self, module: T) -> &mut StdErrLog {
        let to_insert = module.into();
        // If Ok, the module was already found
        if let Err(i) = self.modules.binary_search(&to_insert) {
            self.modules.insert(i, to_insert);
        }
        self
    }

    /// specifiy modules to allow to log to stderr
    pub fn modules<T: Into<String>, I: IntoIterator<Item = T>>(
        &mut self,
        modules: I,
    ) -> &mut StdErrLog {
        for module in modules {
            self.module(module);
        }
        self
    }

    fn log_level_filter(&self) -> LevelFilter {
        if self.quiet {
            LevelFilter::Off
        } else {
            self.verbosity
        }
    }

    fn includes_module(&self, module_path: &str) -> bool {
        // If modules is empty, include all module paths
        if self.modules.is_empty() {
            return true;
        }
        // if a prefix of module_path is in `self.modules`, it must
        // be located at the first location before
        // where module_path would be.
        match self.modules
            .binary_search_by(|module| module.as_str().cmp(&module_path))
        {
            Ok(_) => {
                // Found exact module: return true
                true
            }
            Err(0) => {
                // if there's no item which would be located before module_path, no prefix is there
                false
            }
            Err(i) => module_path.starts_with(&self.modules[i - 1]),
        }
    }

    /// sets the the logger as active
    pub fn init(&self) -> Result<(), log::SetLoggerError> {
        log::set_max_level(self.log_level_filter());
        log::set_boxed_logger(Box::new(self.clone()))
    }
}

impl Default for StdErrLog {
    fn default() -> Self {
        StdErrLog::new()
    }
}

/// creates a new stderr logger
pub fn new() -> StdErrLog {
    StdErrLog::new()
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_default_level() {
        extern crate log;

        super::new().module(module_path!()).init().unwrap();

        assert_eq!(log::Level::Error, log::max_level())
    }
}
