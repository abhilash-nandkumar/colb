use anstyle::{AnsiColor, Color, Style};
use serde::{Deserialize, Serialize};
use std::{
    env,
    io::Write,
    ops::Deref,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

use clap::{Parser, Subcommand};

#[derive(Serialize, Deserialize)]
enum BuildType {
    Debug,
    Release,
    RelWithDebInfo,
}

impl BuildType {
    fn apply(&self, cmd: &mut ArgStack) {
        cmd.arg("--cmake-args");
        let t = match self {
            BuildType::Debug => "Debug",
            BuildType::Release => "Release",
            BuildType::RelWithDebInfo => "RelWithDebInfo",
        };
        cmd.arg(format!("-DCMAKE_BUILD_TYPE={t}"));
    }
}

#[derive(Default)]
struct ArgStack {
    args: Vec<String>,
}

impl ArgStack {
    pub fn arg<S: Into<String>>(&mut self, arg: S) -> &mut Self {
        self.args.push(arg.into());
        self
    }

    fn args<I, S>(&mut self, args: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for arg in args {
            self.arg(arg.into());
        }
    }
}

// Make ArgStack.iter() work
impl Deref for ArgStack {
    type Target = Vec<String>;

    fn deref(&self) -> &Self::Target {
        &self.args
    }
}

struct ColconInvocation {
    args: ArgStack,
    workspace: String,
}

struct BuildVerb {
    args: ArgStack,
    workspace: String,
}

struct BasicVerb {
    args: ArgStack,
    workspace: String,
}

struct ConfiguredBuild {
    args: ArgStack,
    workspace: String,
}

#[derive(Default)]
struct BuildOutput {
    symlink: bool,
    merge: bool,
}

#[derive(Serialize, Deserialize)]
struct EventHandlers {
    desktop_notification: bool,
    console_cohesion: bool,
    summary: bool,
    console_start_end: bool,
}

impl Default for EventHandlers {
    fn default() -> Self {
        Self {
            desktop_notification: false,
            console_cohesion: false,
            summary: true,
            console_start_end: true,
        }
    }
}

impl EventHandlers {
    fn silent() -> Self {
        Self {
            desktop_notification: false,
            console_cohesion: false,
            summary: false,
            console_start_end: false,
        }
    }

    fn compile_logs_only() -> Self {
        let mut res = Self::silent();
        res.console_cohesion = true;
        res
    }

    fn apply(&self, args: &mut ArgStack) {
        args.arg("--event-handlers");
        args.arg(handler_str("summary", self.summary));
        args.arg(handler_str("console_start_end", self.console_start_end));
        args.arg(handler_str("console_cohesion", self.console_cohesion));
        args.arg(handler_str(
            "desktop_notification",
            self.desktop_notification,
        ));
    }
}

#[derive(Serialize, Deserialize)]
struct BuildConfiguration {
    mixins: Vec<String>,
    cmake_args: Vec<String>,
    build_type: BuildType,
    parallel_jobs: Option<u32>,
    event_handlers: EventHandlers,
    build_tests: bool,
}

struct TestConfiguration {
    package: String,
    event_handlers: EventHandlers,
}

struct TestResultConfig {
    package: String,
    verbose: bool,
    all: bool,
}

#[derive(Serialize, Deserialize)]
struct Config {
    upstream: BuildConfiguration,
    package: BuildConfiguration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            upstream: BuildConfiguration::upstream(),
            package: BuildConfiguration::active(),
        }
    }
}

enum What {
    DependenciesFor(String),
    ThisPackage(String),
}

impl ColconInvocation {
    fn new(workspace: &str, log: bool) -> ColconInvocation {
        let mut args = ArgStack::default();
        args.arg("--log-base");
        if log {
            args.arg("log");
        } else {
            args.arg("/dev/null");
        }
        ColconInvocation {
            args,
            workspace: workspace.into(),
        }
    }

    fn build(self, base_setup: &BuildOutput) -> BuildVerb {
        let mut res = BuildVerb {
            args: self.args,
            workspace: self.workspace,
        };
        res.args.arg("build");
        res.args
            .args(["--build-base", "build", "--install-base", "install"]);
        if base_setup.symlink {
            res.args.arg("--symlink-install");
        }
        if base_setup.merge {
            res.args.arg("--merge-install");
        }
        res
    }

    fn test(self, config: &TestConfiguration) -> BasicVerb {
        let mut res = BasicVerb {
            args: self.args,
            workspace: self.workspace,
        };
        // TODO: log is probably needed here?
        res.args.arg("test");
        res.args.arg("--event-handlers");
        config.event_handlers.apply(&mut res.args);
        res.args.args(["--ctest-args", "--output-on-failure"]);
        res.args.args(["--packages-select", &config.package]);
        res
    }

    fn test_result(self, config: &TestResultConfig) -> BasicVerb {
        let mut res = BasicVerb {
            args: self.args,
            workspace: self.workspace,
        };
        // TODO: log is probably needed here?
        res.args.arg("test-result");
        res.args
            .args(["--test-result-base", &format!("build/{}", config.package)]);
        if config.verbose {
            res.args.arg("--verbose");
        }
        if config.all {
            res.args.arg("--all");
        }
        res
    }
}

fn handler_str(name: &str, enabled: bool) -> String {
    format!("{name}{}", if enabled { "+" } else { "-" })
}

fn cmake_arg(name: &str, value: &str) -> String {
    format!("-D{name}={value}")
}

impl BuildConfiguration {
    const DEFAULT_MIXINS: &'static [&'static str] =
        &["compile-commands", "ninja", "mold", "ccache"];
    fn upstream() -> BuildConfiguration {
        BuildConfiguration {
            mixins: Self::DEFAULT_MIXINS
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<String>>(),
            cmake_args: vec![],
            build_type: BuildType::Debug,
            parallel_jobs: Some(8),
            event_handlers: EventHandlers::default(),
            build_tests: false,
        }
    }

    fn active() -> BuildConfiguration {
        BuildConfiguration {
            mixins: Self::DEFAULT_MIXINS
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<String>>(),
            cmake_args: vec![],
            build_type: BuildType::Debug,
            parallel_jobs: Some(8),
            event_handlers: EventHandlers::compile_logs_only(),
            build_tests: true,
        }
    }
}

impl BuildVerb {
    fn configure(self, config: &BuildConfiguration) -> ConfiguredBuild {
        let mut res = ConfiguredBuild {
            args: self.args,
            workspace: self.workspace,
        };
        if let Some(n) = config.parallel_jobs {
            let n_arg = format!("{}", n);
            res.args
                .args(["--executor", "parallel", "--parallel-workers", &n_arg]);
        }
        config.event_handlers.apply(&mut res.args);
        if !config.mixins.is_empty() {
            res.args.arg("--mixin").args(config.mixins.iter());
        }
        res.args.arg("--cmake-args");
        res.args.arg(cmake_arg(
            "BUILD_TESTING",
            if config.build_tests { "ON" } else { "OFF" },
        ));
        res.args.args(config.cmake_args.iter());
        config.build_type.apply(&mut res.args);
        res
    }
}

const DECO: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::BrightBlack)));
const HEADER: Style = Style::new().bold().fg_color(Some(Color::Ansi(AnsiColor::BrightBlue)));

macro_rules! header {
    ($($l:tt)*) => {
        print!("{DECO}┌[{DECO:#} {HEADER}");
        print!($($l)*);
        println!("{HEADER:#} {DECO}]{DECO:#}");
    };
}
macro_rules! context {
    ($($l:tt)*) => {
        print!("{DECO}└>{DECO:#} ");
        println!($($l)*);
    };
}

fn print_command(command: &Command) {
    print!(
        "{DECO}└>{DECO:#} {}",
        command.get_program().to_string_lossy()
    );
    for arg in command.get_args() {
        print!(" {}", arg.to_string_lossy());
    }
    println!();
    divider();
}

fn divider() {
    println!("{DECO}[ \\ \\ \\{DECO:#} Output {DECO}/ / / ]{DECO:#}");
}

impl ConfiguredBuild {
    fn run(&self, what: &What) -> ExitStatus {
        let mut cmd = Command::new("colcon");
        cmd.current_dir(&self.workspace);
        cmd.args(self.args.iter());
        match what {
            What::DependenciesFor(package) => {
                cmd.arg("--packages-up-to").arg(package);
                cmd.arg("--packages-skip").arg(package);
            }
            What::ThisPackage(package) => {
                cmd.arg("--packages-select").arg(package);
            }
        }
        print_command(&cmd);
        cmd.status().expect("'colcon' not found")
    }
}

impl BasicVerb {
    fn run(&self) -> ExitStatus {
        let mut cmd = Command::new("colcon");
        cmd.current_dir(&self.workspace);
        cmd.args(self.args.iter());
        print_command(&cmd);
        cmd.status().expect("'colcon' not found")
    }
}

fn ninja_build_target(workspace: &str, package: &str, target: &str) -> ExitStatus {
    let mut cmd = Command::new("ninja");
    cmd.arg("-C");
    cmd.arg(format!("{workspace}/build/{package}"));
    cmd.arg(target);
    print_command(&cmd);
    cmd.status().expect("'ninja' not found")
}

fn run_single_ctest(workspace: &str, package: &str, target: &str) -> ExitStatus {
    let mut cmd = Command::new("ctest");
    cmd.arg("--test-dir");
    cmd.arg(format!("{workspace}/build/{package}"));
    cmd.arg("--output-on-failure");
    cmd.arg("-R");
    cmd.arg(format!("^{target}$"));
    print_command(&cmd);
    cmd.status().expect("'ctest' not found")
}

fn contains_marker(path: &Path, markers: &[&str]) -> bool {
    for m in markers {
        let candidate = path.join(m);
        if let Ok(x) = candidate.try_exists() {
            if x {
                return true;
            }
        }
    }
    false
}

/// Search upward, and if we hit a package.xml, use that folder name as the package
fn find_upwards(markers: &[&str]) -> Option<PathBuf> {
    let mut cwd = env::current_dir().and_then(|p| p.canonicalize()).ok();
    while let Some(p) = cwd {
        if contains_marker(&p, markers) {
            return Some(p.to_path_buf());
        }
        cwd = p.parent().map(|x| x.to_path_buf());
    }
    None
}

fn package_or(package: Option<String>) -> Option<String> {
    if package.is_some() {
        return package;
    }
    find_upwards(&["package.xml"])
        .and_then(|f| f.file_name().map(|n| n.to_string_lossy().to_string()))
}

const COLB_CONFIG_FILENAME: &str = ".colb.toml";

fn detect_workspace() -> Option<String> {
    find_upwards(&["build", COLB_CONFIG_FILENAME]).map(|n| n.to_string_lossy().to_string())
}

/// A colcon wrapper for faster change compile test cycles
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[arg(short, long)]
    workspace: Option<String>,

    #[command(subcommand)]
    verb: Verbs,
}

#[derive(Subcommand)]
enum Verbs {
    /// Write default configuration file
    Init {
        /// Wheter to overwrite existing config files
        #[arg(short, long, default_value_t = false)]
        force: bool,
    },
    /// Build a package
    Build {
        /// The package to build (default: current directory)
        package: Option<String>,

        /// Whether to skip rebuilding dependencies
        #[arg(short, long, default_value_t = false)]
        skip_dependencies: bool,
    },

    /// Run tests for a package
    Test {
        /// The package to test (default: current directory)
        package: Option<String>,

        /// Build and run only this test (default: run all tests)
        #[arg(short, long)]
        test: Option<String>,

        /// Don't rebuild the package
        #[arg(short, long, default_value_t = false)]
        skip_rebuild: bool,

        /// Rebuild dependencies of package
        #[arg(short, long, default_value_t = false)]
        rebuild_dependencies: bool,
    },
}

fn exit_on_error(status: ExitStatus) {
    match status.code() {
        Some(0) => {}
        Some(code) => {
            std::process::exit(code);
        }
        None => {
            std::process::exit(-1);
        }
    }
}

fn main() {
    let exit_on_not_found = || {
        eprintln!("Could not detect package, try specifying it explicitly!");
        std::process::exit(-1);
    };

    let config_file_err = |err| {
        eprintln!("Could not open config file: {}", err);
        std::process::exit(-1);
    };

    let config_parse_err = |err| {
        eprintln!("Could not parse config file: {}", err);
        std::process::exit(-1);
    };

    let cli = Cli::parse();
    let ws = cli
        .workspace
        .or_else(detect_workspace)
        .unwrap_or(".".into());
    let ws_str = Path::new(&ws)
        .canonicalize()
        .map(|x| x.to_string_lossy().to_string())
        .unwrap_or(ws.clone());
    let cfg_file_path = Path::new(&ws).join(COLB_CONFIG_FILENAME);
    header!("Workspace");
    let config = if cfg_file_path.exists() {
        context!(
            "{} (Using configuration from {})",
            &ws_str,
            COLB_CONFIG_FILENAME
        );
        let data = std::fs::read_to_string(&cfg_file_path)
            .map_err(config_file_err)
            .unwrap();
        toml::from_str::<Config>(&data)
            .map_err(config_parse_err)
            .unwrap()
    } else {
        context!("{} (Unconfigured)", &ws_str);
        Config::default()
    };
    match &cli.verb {
        Verbs::Init { force } => {
            if cfg_file_path.exists() && !force {
                println!(
                    "Will not overwrite '{}' without --force",
                    cfg_file_path.to_string_lossy()
                );
                std::process::exit(-1);
            }
            match std::fs::File::create(&cfg_file_path) {
                Ok(mut f) => {
                    let res = f.write_all(
                        toml::to_string_pretty(&Config::default())
                            .expect("Default config should be serializable")
                            .as_bytes(),
                    );
                    if res.is_ok() {
                        println!(
                            "Initialized default configuration at '{}'",
                            &cfg_file_path.to_string_lossy()
                        );
                        std::process::exit(0);
                    }
                    eprintln!(
                        "Could not cerate '{}': {}",
                        cfg_file_path.to_string_lossy(),
                        res.unwrap_err()
                    );
                    std::process::exit(-1);
                }
                Err(e) => {
                    eprintln!(
                        "Could not cerate '{}': {}",
                        cfg_file_path.to_string_lossy(),
                        e
                    );
                    std::process::exit(-1);
                }
            }
        }
        Verbs::Build {
            package,
            skip_dependencies,
        } => {
            let package = package_or(package.clone())
                .or_else(exit_on_not_found)
                .expect("should have exited");
            if !skip_dependencies {
                header!("Building dependencies for '{}'", package);
                let status = ColconInvocation::new(&ws, false)
                    .build(&BuildOutput::default())
                    .configure(&config.upstream)
                    .run(&What::DependenciesFor(package.clone()));
                exit_on_error(status);
            }
            header!("Building '{package}'");
            let status = ColconInvocation::new(&ws, false)
                .build(&BuildOutput::default())
                .configure(&config.package)
                .run(&What::ThisPackage(package.clone()));
            exit_on_error(status);
        }

        Verbs::Test {
            package,
            test,
            skip_rebuild,
            rebuild_dependencies,
        } => {
            let package = package_or(package.clone())
                .or_else(exit_on_not_found)
                .expect("should have exited");
            if *rebuild_dependencies && !skip_rebuild {
                header!("Building dependencies for '{}'", package);
                let status = ColconInvocation::new(&ws, false)
                    .build(&BuildOutput::default())
                    .configure(&config.upstream)
                    .run(&What::DependenciesFor(package.clone()));
                exit_on_error(status);
                if test.is_some() {
                    header!("Building '{package}'");
                    let status = ColconInvocation::new(&ws, false)
                        .build(&BuildOutput::default())
                        .configure(&config.package)
                        .run(&What::ThisPackage(package.clone()));
                    exit_on_error(status);
                }
            }
            if !skip_rebuild {
                if let Some(test) = test {
                    header!("Building test '{test}' in '{package}'");
                    let status = ninja_build_target(&ws, &package, test);
                    exit_on_error(status);
                } else {
                    header!("Building '{package}'");
                    let status = ColconInvocation::new(&ws, false)
                        .build(&BuildOutput::default())
                        .configure(&config.package)
                        .run(&What::ThisPackage(package.clone()));
                    exit_on_error(status);
                }
            }
            if let Some(test) = test {
                header!("Running test '{test}' in '{package}'");
                let status = run_single_ctest(&ws, &package, test);
                exit_on_error(status);
            } else {
                header!("Running tests for '{package}'");
                let status = ColconInvocation::new(&ws, true)
                    .test(&TestConfiguration {
                        package: package.clone(),
                        event_handlers: EventHandlers::silent(),
                    })
                    .run();
                exit_on_error(status);
                header!("Test results for '{package}'");
                let status = ColconInvocation::new(&ws, false)
                    .test_result(&TestResultConfig {
                        package: package.clone(),
                        verbose: true,
                        all: true,
                    })
                    .run();
                exit_on_error(status);
            }
        }
    }
}
