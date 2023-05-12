use crate::server::{Algorithm, Statistics};
use crate::shared::{Bundle, BundleConfig};
use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use comfy_table::*;
use console::style;
use git2::{Repository, RepositoryOpenFlags};
use indicatif::{
    FormattedDuration, HumanBytes, HumanDuration, ProgressBar, ProgressState, ProgressStyle,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env::current_dir;
use std::fmt::Write;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::Duration;
use ulid::Ulid;

const LAUNCH_FILE_NAME: &str = "launch.json";

#[derive(Subcommand)]
pub enum Command {
    /// Bootstraps the current folder for deployment
    Init(InitOptions),

    /// Shows a list of all current deployments
    #[clap(alias("ls"))]
    List {
        #[arg(short, long, env = "LAUNCH_ENDPOINT")]
        endpoint: String,
    },

    /// Launches it (pushes the current repository)
    It {
        #[arg(short, long, env = "LAUNCH_ENDPOINT")]
        endpoint: String,
    },

    /// Removes the current repository if it is deployed
    Deorbit {
        #[arg(short, long, env = "LAUNCH_ENDPOINT")]
        endpoint: String,

        /// Deployment to delete, will be inferred from the current dir if left blank
        id: Option<Ulid>,
    },
}

#[derive(Args)]
pub struct InitOptions {
    name: String,
    domain: String,

    /// Location of the build root, usually something like `dist` or `build`. Relative to project root!
    #[arg(short, long)]
    root: Option<PathBuf>,

    /// Path to a file which is served if nothing else matches. Useful for SPAs.
    #[arg(short, long)]
    fallback: Option<String>,

    /// Reinitialize the config, disconnecting it from deployed instances
    #[arg(long)]
    force: bool,
}

#[derive(Serialize, Deserialize)]
struct LaunchConfig {
    id: Ulid,
    root: PathBuf,

    #[serde(flatten)]
    bundle: BundleConfig,
}

impl LaunchConfig {
    fn new(options: InitOptions) -> Result<Self> {
        let root = options.root.unwrap_or(".".into());

        Ok(Self {
            id: Ulid::new(),
            root,
            bundle: BundleConfig {
                name: options.name,
                domain: options.domain,
                compress: vec![
                    "html".into(),
                    "js".into(),
                    "json".into(),
                    "css".into(),
                    "woff".into(),
                    "woff2".into(),
                ],
                fallback: options.fallback,
            },
        })
    }
}

pub fn run(command: Command) -> Result<()> {
    match command {
        Command::List { endpoint } => list(&endpoint),
        Command::Init(c) => init(c),
        Command::It { endpoint } => launch(&endpoint),
        Command::Deorbit { endpoint, id } => delete(&endpoint, id),
    }
}

fn init(options: InitOptions) -> Result<()> {
    let path = find_project_root()?.join(LAUNCH_FILE_NAME);
    if path.exists() && !options.force {
        bail!("launch config already present, use --force if you want to recreate it!");
    }

    let config = LaunchConfig::new(options)?;
    let mut file = File::create(path)?;
    serde_json::to_writer_pretty(&mut file, &config)?;

    Ok(())
}

fn list(endpoint: &str) -> Result<()> {
    let config = load_config();
    let active_id = config.ok().map(|c| c.id);

    let mut bundles = ureq::get(endpoint)
        .call()
        .context("http req failed")?
        .into_json::<HashMap<Ulid, Bundle>>()
        .context("failed to deserialize response")?
        .into_iter()
        .collect::<Vec<_>>();

    bundles.sort_by_key(|(id, _)| *id);

    let mut table = Table::new();

    table
        .load_preset("     ═╪            ")
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new(""),
            Cell::new("Name"),
            Cell::new("Domain").set_alignment(CellAlignment::Center),
            Cell::new("Size").set_alignment(CellAlignment::Right),
            Cell::new("Savings").set_alignment(CellAlignment::Right),
        ]);

    for (id, bundle) in bundles {
        match bundle {
            Bundle::Active { config, stats } => {
                let mut id_cell = Cell::new(id);

                if Some(id) == active_id {
                    id_cell = id_cell.add_attribute(Attribute::Bold);
                } else {
                    id_cell = id_cell.add_attribute(Attribute::Dim);
                }

                let brotli = if let Some(compressed) = stats.compressed.get(&Algorithm::Brotli) {
                    let percentage =
                        ((stats.compressible - compressed) as f64 / stats.size as f64) * 100.0;
                    format!("{:0>2.2}%", percentage)
                } else {
                    "100%".into()
                };

                table.add_row(vec![
                    id_cell,
                    Cell::new(config.name).fg(Color::Green),
                    Cell::new(config.domain)
                        .fg(Color::Cyan)
                        .set_alignment(CellAlignment::Right),
                    Cell::new(HumanBytes(stats.size)).set_alignment(CellAlignment::Right),
                    Cell::new(brotli).set_alignment(CellAlignment::Right),
                ]);
            }
            Bundle::Failed { error } => {
                table.add_row(vec![id.to_string(), error]);
            }
        }
    }

    println!("\n{table}\n");

    Ok(())
}

fn launch(endpoint: &str) -> Result<()> {
    println!(
        "{} 🪄  Designing schematics...",
        style("[1/4]").bold().dim()
    );

    let config = load_config().context("failed to find load config")?;
    let root = find_build_root(&config).context("failed to find build root")?;

    let temp = temp_dir::TempDir::new().context("failed to create temp dir")?;
    let path = temp.child("launch.bundle.tar");
    let path_meta = temp.child("launch.config");

    std::fs::write(&path_meta, serde_json::to_string(&config.bundle)?)
        .context("failed to write metadata")?;

    println!("{} 🛠️  Assembling rocket...", style("[2/4]").bold().dim());

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .truncate(true)
        .create(true)
        .open(path)
        .context("failed to create archive file")?;

    {
        let mut buf_wrt = BufWriter::new(&mut file);
        let mut builder = tar::Builder::new(&mut buf_wrt);

        builder
            .append_path_with_name(path_meta, "./launch.config")
            .context("failed to add launch config to archive")?;

        builder
            .append_dir_all(".", root)
            .context("failed to add files to archive")?;

        builder.finish().context("failed to finalise archive")?;
    }

    file.seek(SeekFrom::Start(0))
        .context("failed to seek through archive")?;

    println!(
        "         {} {}",
        style("Takeoff mass is").dim(),
        style(HumanBytes(file.metadata()?.len())).dim().bold(),
    );

    println!(
        "{} ⏰ Starting final countdown...",
        style("[3/4]").bold().dim()
    );

    let mut reader = CountingReader::new(&mut file)?;
    let req_path = format!("{endpoint}/bundle/{}", config.id);
    let res = ureq::post(&req_path).send(&mut reader);
    reader.finish();

    match res {
        Ok(response) => {
            let stats: Statistics = serde_json::from_reader(response.into_reader())?;

            if let Some(compressed) = stats.compressed.get(&Algorithm::Brotli) {
                let percentage_total =
                    ((stats.compressible - compressed) as f64 / stats.size as f64) * 100.0;
                let percentage_burned =
                    (1.0 - *compressed as f64 / stats.compressible as f64) * 100.0;

                println!(
                    "         {} {}{}",
                    style("Burned").dim(),
                    style((percentage_burned * 100.0).round() / 100.0)
                        .dim()
                        .bold(),
                    style("% of fuel").dim()
                );

                println!(
                    "         {} {}{}",
                    style("Lost").dim(),
                    style((percentage_total * 100.0).round() / 100.0)
                        .dim()
                        .bold(),
                    style("% of total mass").dim()
                );
            }

            println!("{}", include_str!("./liftoff.txt"));

            let url = format!("https://{}", config.bundle.domain);
            println!(
                "Visit \x1b]8;;{}\x07{}\x1b]8;;\x07 to check the mission!",
                url, url
            );

            Ok(())
        }
        Err(ureq::Error::Status(code, response)) => Err(anyhow!(
            "Uh, oh ... we had a rapid, unscheduled disassembly 😳\n\t({} — {})",
            code,
            response.into_string().unwrap_or_default()
        )),
        Err(error) => Err(error).context("failed to send request"),
    }

    // TODO Verify deployment
}

fn delete(endpoint: &str, id: Option<Ulid>) -> Result<()> {
    let id = id
        .or_else(|| {
            let config = load_config().ok()?;
            Some(config.id)
        })
        .ok_or(anyhow!("could not infer deployment id"))?;

    ureq::delete(&format!("{endpoint}/bundle/{}", id))
        .call()
        .context("failed to delete deployment")?;

    Ok(())
}

fn load_config() -> Result<LaunchConfig> {
    let path = find_project_root()?.join(LAUNCH_FILE_NAME);
    let file = File::open(path)?;
    let config: LaunchConfig = serde_json::from_reader(&file)?;
    Ok(config)
}

fn find_build_root(config: &LaunchConfig) -> Result<PathBuf> {
    Ok(find_project_root()?.join(&config.root))
}

fn find_project_root() -> Result<PathBuf> {
    let cwd = current_dir()?;
    let repo = Repository::open_ext::<_, PathBuf, _>(cwd, RepositoryOpenFlags::empty(), vec![])?;

    Ok(repo
        .path()
        .parent()
        .ok_or_else(|| anyhow!("git repo has no parent directory"))?
        .to_path_buf())
}

struct CountingReader<'f> {
    file: &'f mut File,
    bar: ProgressBar,
    read_finished: bool,
}

impl<'f> CountingReader<'f> {
    fn new(file: &'f mut File) -> Result<Self> {
        let bar = ProgressBar::new(file.metadata()?.len());

        bar.set_style(
            ProgressStyle::with_template(
                "\n{spinner:.green} [{smoothed_eta}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes}",
            )?
            .with_key(
                "smoothed_eta",
                |s: &ProgressState, w: &mut dyn Write| match (s.pos(), s.len()) {
                    (pos, Some(len)) if pos > 0 => write!(
                        w,
                        "{:#}",
                        FormattedDuration(Duration::from_millis(
                            (s.elapsed().as_millis() * (len as u128 - pos as u128) / (pos as u128))
                                as u64
                        ))
                    )
                    .unwrap(),
                    _ => write!(w, "-").unwrap(),
                },
            ),
        );

        Ok(Self {
            bar,
            file,
            read_finished: false,
        })
    }

    fn close_read(&mut self) {
        self.read_finished = true;
        self.bar.finish_and_clear();

        println!(
            "         {} {}",
            style("Countdown took").dim(),
            style(HumanDuration(self.bar.elapsed())).dim().bold(),
        );

        self.bar = ProgressBar::new_spinner();
        self.bar.enable_steady_tick(Duration::from_millis(50));
        self.bar.set_style(
            ProgressStyle::with_template("{prefix:.bold.dim} {spinner} {wide_msg}")
                .expect("progress style is invalid"),
        );
        self.bar
            .set_prefix(style("[4/4] ").bold().dim().to_string());
        self.bar.set_message("Main engine ignition...");
    }

    fn finish(&self) {
        self.bar.finish_and_clear();
        println!("{} 🚀 Main engine ignition...", style("[4/4]").bold().dim());
    }
}

impl<'f> Read for CountingReader<'f> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.file.read(buf)?;
        self.bar.inc(read as u64);

        if !self.read_finished && self.bar.position() == self.bar.length().unwrap() {
            self.close_read();
        }

        Ok(read)
    }
}
