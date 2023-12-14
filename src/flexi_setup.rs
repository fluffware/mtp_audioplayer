use clap::{Arg, ArgMatches, Command};
use flexi_logger::{self, Cleanup, Criterion, FileSpec, LoggerHandle, Naming};
use std::error::Error;

pub fn add_flexi_args<'a>(app_args: Command<'a>) -> Command<'a> {
    let app_args = app_args.arg(
        Arg::new("log_file")
            .long("log_file")
            .value_name("PATTERN")
            .help("Write log to files named by this pattern. stderr or journald if not set"),
    );
    let app_args = app_args.arg(
        Arg::new("log_file_size")
            .long("log_file_size")
            .value_name("SIZE")
            .default_value("1048576")
            .value_parser(clap::value_parser!(u64))
            .help("Maximum size for log files"),
    );
    let app_args = app_args.arg(
        Arg::new("log_file_count")
            .long("log_file_count")
            .value_name("COUNT")
            .default_value("10")
            .value_parser(clap::value_parser!(usize))
            .help("Maximum number of log files"),
    );
    app_args
}

pub fn setup_flexi_loggger(args: &ArgMatches) -> Result<LoggerHandle, Box<dyn Error>> {
    let mut logger = flexi_logger::Logger::try_with_env_or_str("info")?;
    if let Some(filepath) = args.get_one::<String>("log_file") {
        let spec = FileSpec::try_from(filepath)?;
        logger = logger.log_to_file(spec);
    }
    logger = logger.format(flexi_logger::detailed_format);
    let size = args.try_get_one::<u64>("log_file_size")?.unwrap();
    let criterion = Criterion::Size(*size);
    let count = args.try_get_one::<usize>("log_file_count")?.unwrap();
    let cleanup = Cleanup::KeepLogFiles(*count);
    logger = logger.rotate(criterion, Naming::Timestamps, cleanup);

    Ok(logger.start()?)
}
