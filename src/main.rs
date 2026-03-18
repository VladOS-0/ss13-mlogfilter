use std::{
    fs::{OpenOptions, create_dir_all, read_to_string},
    io::{Read, Write, stdin},
    path::{Path, PathBuf},
    process::exit,
    time::Instant,
};

use clap::{Parser, ValueEnum};

use crate::config::Config;

mod config;

/// Simple CLI utility to filter Space Station 13 saved chat logs
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Paths to chat log files to filter
    paths: Vec<PathBuf>,

    /// Paths to output files. Defaults to "{out_dir}/filtered_{INPUT FILE NAME}". out_dir defaults to current working
    /// directory the program's working directory. Missing directories in the path will be created recursively. If more
    /// paths than outputs were provided, missing outputs will be set to default. If more outputs than paths
    /// were provided, excessive outputs will be ignored.
    #[arg(short, long, value_name = "FILES")]
    outputs: Vec<PathBuf>,

    /// Path to the directory, which will be considered base for default outputs. Missing directories in the path will be
    /// created recursively.
    #[arg(short = 'O', long, value_name = "DIR")]
    out_dir: Option<PathBuf>,

    /// How program will use standard input. Default is "none"
    #[arg(long)]
    stdin: Option<StdinMode>,

    /// Exits the program if failed to filter one or more paths
    #[arg(long)]
    strict: bool,

    /// Allow overwrite of output files
    #[arg(long)]
    overwrite: bool,

    /// Match case
    #[arg(long)]
    match_case: bool,

    /// Treat include & exclude patterns as regexes
    #[arg(long)]
    regex: bool,

    /// Patterns that has to be included in the output
    #[arg(short, long)]
    include: Option<String>,

    /// Patterns that has to be excluded from the output
    #[arg(short, long)]
    exclude: Option<String>,

    /// Path to a config file
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Default)]
enum StdinMode {
    /// Standard input will be ignored
    #[default]
    None,
    /// Filter will try to read one chat log from standard input to filter. If "--outputs" / "-o" is set, the first output path will be taken
    /// for stdin log. Otherwise it will be set to "filtered_stdin.html"
    Log,
    /// Filter will try to read paths to chat logs separated with whitespaces from standard input
    Path,
}

fn main() {
    let start = Instant::now();

    let mut cli = Cli::parse();

    let config: Config;

    if let Some(config_path) = cli.config {
        config = Config::load(&config_path).unwrap_or_else(|err| {
            eprintln!(
                "Failed to load config from {}: {}",
                config_path.to_string_lossy(),
                err
            );
            exit(1);
        });
    } else {
        config = Config::from_args(cli.regex, cli.include, cli.exclude, cli.match_case)
            .unwrap_or_else(|err| {
                eprintln!("Failed to parse arguments: {}", err);
                exit(1)
            });
    }

    // Source (stdin or path to the log), path to output and log string
    let mut logs: Vec<(String, PathBuf, String)> = Vec::with_capacity(cli.paths.len());

    match cli.stdin.unwrap_or_default() {
        StdinMode::None => {}
        StdinMode::Log => {
            let mut log: String = String::new();
            stdin().read_to_string(&mut log).unwrap_or_else(|err| {
                eprintln!("Failed to read from standard input: {}", err);
                exit(1);
            });
            logs.push((
                "stdin".to_string(),
                get_path_for_output(0, &cli.outputs, None, &cli.out_dir),
                log,
            ));
        }
        StdinMode::Path => {
            let mut stdin_paths: String = String::new();
            stdin()
                .read_to_string(&mut stdin_paths)
                .unwrap_or_else(|err| {
                    eprintln!("Failed to read from standard input: {}", err);
                    exit(1);
                });
            let mut stdin_paths: Vec<PathBuf> = stdin_paths
                .split_whitespace()
                .map(|path| path.into())
                .collect();
            println!("Parsed {} paths from standard input.", stdin_paths.len());
            cli.paths.append(&mut stdin_paths);
        }
    }

    for (index, log_path) in cli.paths.iter().enumerate() {
        let index = match cli.stdin {
            Some(StdinMode::Log) => index + 1,
            _ => index,
        };

        let output_path = get_path_for_output(index, &cli.outputs, Some(log_path), &cli.out_dir);

        match read_to_string(log_path) {
            Ok(log) => {
                logs.push((log_path.to_string_lossy().into(), output_path, log));
            }
            Err(err) => {
                eprintln!(
                    "Failed to read input path {}: {}",
                    log_path.to_string_lossy(),
                    err
                );
                continue;
            }
        }
    }

    if logs.is_empty() {
        eprintln!("No valid logs were provided. Use \"--help\" argument for help.");
        exit(1)
    }

    for (source, output_path, log) in &logs {
        let this_path_start = Instant::now();

        match process_log(log, output_path, &config, cli.overwrite) {
            Ok(()) => {
                println!(
                    "Filtered chat log from {} to {} in {}ms",
                    source,
                    output_path.to_string_lossy(),
                    this_path_start.elapsed().as_millis()
                );
            }
            Err(err) => {
                eprintln!("Failed to process {}: {}", source, err);
                if cli.strict {
                    eprintln!("Encountered error in strict mode. Exiting...");
                    exit(1)
                } else {
                    continue;
                }
            }
        }
    }

    println!(
        "Filtered {} logs in {}ms",
        logs.len(),
        start.elapsed().as_millis()
    );
}

fn get_path_for_output(
    index: usize,
    outputs: &[PathBuf],
    path: Option<&Path>,
    base_dir: &Option<PathBuf>,
) -> PathBuf {
    if let Some(output) = outputs.get(index) {
        return output.clone();
    }
    let base_dir = match &base_dir {
        Some(dir) => dir.to_string_lossy().trim_end_matches("/").to_string(),
        None => ".".to_string(),
    };
    let file_name = match path {
        Some(path) => path
            .file_name()
            .map(|file_name| file_name.to_string_lossy())
            .unwrap_or(format!("file_name_error{}", index).into()),
        None => std::borrow::Cow::Borrowed("stdin"),
    };

    PathBuf::from(format!("{}/filtered_{}", base_dir, file_name))
}

fn process_log(
    log: &String,
    output_path: &PathBuf,
    config: &Config,
    overwrite: bool,
) -> Result<(), anyhow::Error> {
    let filtered_chat_log = filter_chat_log(log, config).unwrap_or_else(|err| {
        eprintln!("filter error: {}", err);
        exit(1);
    });

    let parent_dir = output_path.parent().ok_or(anyhow::format_err!(
        "invalid output path {}",
        output_path.to_string_lossy()
    ))?;
    create_dir_all(parent_dir).map_err(|err| {
        anyhow::format_err!(
            "failed to create parent directories for {}: {}",
            output_path.to_string_lossy(),
            err
        )
    })?;

    let mut output_file = OpenOptions::new()
        .write(true)
        .create_new(!overwrite)
        .create(overwrite)
        .truncate(overwrite)
        .open(output_path)
        .unwrap_or_else(|err| {
            eprintln!(
                "error while creating an output file {}: {}",
                output_path.to_string_lossy(),
                err
            );
            exit(1);
        });

    output_file
        .write_all(filtered_chat_log.as_bytes())
        .unwrap_or_else(|err| {
            eprintln!(
                "error while writing to an output file in {}: {}",
                output_path.to_string_lossy(),
                err
            );
            exit(1);
        });

    Ok(())
}

fn filter_chat_log(chat_log: &String, config: &Config) -> Result<String, anyhow::Error> {
    let mut output = String::with_capacity(chat_log.len());
    let parts: Vec<&str> = chat_log.split_inclusive("<div class=\"Chat\">").collect();
    if parts.len() != 2 {
        Err(anyhow::format_err!(
            "Expected 1 \"<div class=\"Chat\">\", but found {}",
            parts.len() - 1
        ))?
    }
    output.push_str(parts[0]);

    let chat_messages = parts[1].replace("</div>\n</body>\n</html>", "");

    for message in chat_messages.split_inclusive("<div class=\"ChatMessage\"") {
        if config.matches(message)? {
            output.push_str(message);
        }
    }

    output.push_str("</div>\n</body>\n</html>");

    Ok(output)
}
