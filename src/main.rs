
#[macro_use]
extern crate lazy_static;
extern crate regex;
extern crate termion;
extern crate directories;

mod app;
mod commands;
mod external;
mod flat_tree;
mod tree_build;
mod input;
mod status;
mod tree_views;
mod verbs;

use app::App;
use std::env;
use std::path::{PathBuf};
use std::io;
use tree_build::{TreeBuilder};
use directories::{ProjectDirs};

const SHOW_APP: bool = true;

fn main() -> io::Result<()> {
    if let Some(proj_dirs) = ProjectDirs::from("org", "dystroy",  "btree") {
        println!("conf dir: {:?}", proj_dirs.config_dir());
    }

    let args: Vec<String> = env::args().collect();
    let path = match args.len() >= 2 {
        true    => PathBuf::from(&args[1]),
        false   => env::current_dir()?,
    };
    if SHOW_APP {
        let mut app = App::new()?;
        app.push(path)?;
        app.run()?;
    } else {
        let tree = TreeBuilder::from(path)?.build(80)?;
        println!("{:?}", tree);
    }
    Ok(())
}