use std::sync::LazyLock;

static CLI: LazyLock<Cli> = LazyLock::new(|| {
    let mut cli = Cli {
        world_path: None,
        extract_assets: false,
    };

    let mut args = std::env::args();
    args.next();

    for (i, arg) in args.enumerate() {
        if i == 0 {
            if !arg.starts_with("-") {
                cli.world_path = Some(arg);
                continue;
            }
        }

        if arg == "--extract-assets" {
            cli.extract_assets = true;
        }
    }

    cli
});

pub struct Cli {
    world_path: Option<String>,
    extract_assets: bool,
}

impl Cli {
    pub fn world_path() -> Option<&'static String> {
        CLI.world_path.as_ref()
    }

    pub fn extract_assets() -> bool {
        CLI.extract_assets
    }
}
