use std::{
    io,
    ops::Deref,
    process::{Command, ExitStatus},
};

enum BuildType {
    Debug,
    Release,
    RelWithDebInfo,
}

impl BuildType {
    fn apply(&self, cmd: &mut ArgStack) {
        cmd.arg("--cmake-args");
        cmd.arg(match self {
            BuildType::Debug => "Debug",
            BuildType::Release => "Release",
            BuildType::RelWithDebInfo => "RelWithDebInfo",
        });
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
}

struct ConfiguredBuild {
    args: ArgStack,
}

#[derive(Default)]
struct BuildOutput {
    symlink: bool,
    merge: bool,
}

struct BuildConfiguration {
    mixins: Vec<String>,
    cmake_args: Vec<String>,
    build_type: BuildType,
    parallel_jobs: Option<u32>,
    desktop_notify: bool,
    console_cohesion: bool,
    build_tests: bool,
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
            args.arg(format!("{workspace}/log"));
        } else {
            args.arg("/dev/null");
        }
        ColconInvocation {
            args,
            workspace: workspace.into(),
        }
    }

    fn build(self, base_setup: &BuildOutput) -> BuildVerb {
        let mut res = BuildVerb { args: self.args };
        res.args.arg("build");
        res.args
            .arg("--build-base")
            .arg(format!("{}/build", self.workspace));
        res.args
            .arg("--install-base")
            .arg(format!("{}/install", self.workspace));
        if base_setup.symlink {
            res.args.arg("--symlink-install");
        }
        if base_setup.merge {
            res.args.arg("--merge-install");
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
            desktop_notify: false,
            console_cohesion: false,
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
            desktop_notify: true,
            console_cohesion: true,
            build_tests: true,
        }
    }
}

impl BuildVerb {
    fn configure(self, config: &BuildConfiguration) -> ConfiguredBuild {
        let mut res = ConfiguredBuild { args: self.args };
        if let Some(n) = config.parallel_jobs {
            let n_arg = format!("{}", n);
            res.args
                .args(["--executor", "parallel", "--parallel-workers", &n_arg]);
        }
        res.args.arg("--event-handlers");
        res.args
            .arg(handler_str("desktop_notification", config.desktop_notify));
        res.args
            .arg(handler_str("console_cohesion", config.console_cohesion));
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

impl ConfiguredBuild {
    fn run(&self, what: &What) -> io::Result<ExitStatus> {
        let mut cmd = Command::new("colcon");
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
        println!("{:?}", cmd);
        cmd.status()
    }
}

fn build_upstream_and_package(package: &str) -> io::Result<ExitStatus> {
    let build_output = BuildOutput::default();
    let upstream = ColconInvocation::new(".", false)
        .build(&build_output)
        .configure(&BuildConfiguration::upstream());

    let this = ColconInvocation::new(".", false)
        .build(&build_output)
        .configure(&BuildConfiguration::active());

    let status = upstream.run(&What::DependenciesFor(package.into()))?;
    match status.code() {
        Some(0) => this.run(&What::ThisPackage(package.into())),
        _ => Ok(status),
    }
}

fn main() {
    let status = build_upstream_and_package("test").expect("'colcon' not found");
    if let Some(code) = status.code() {
        std::process::exit(code);
    }
    std::process::exit(-1);
}
