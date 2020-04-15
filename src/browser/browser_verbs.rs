use {
    crate::{
        app::{
            AppContext,
            AppStateCmdResult,
        },
        command::Command,
        errors::ProgramError,
        flat_tree::Tree,
        help::HelpState,
        print,
        screens::Screen,
        task_sync::Dam,
        tree_options::TreeOptions,
        verb::{
            Internal,
            Verb,
            VerbExecutor,
            VerbExecution,
            VerbInvocation,
        },
    },
    directories::UserDirs,
    std::path::PathBuf,
    super::*,
};

fn focus_path(
    path: PathBuf,
    screen: &mut Screen,
    tree: &Tree,
    in_new_panel: bool,
) -> AppStateCmdResult {
    AppStateCmdResult::from_optional_state(
        BrowserState::new(
            path,
            tree.options.clone(),
            screen,
            &Dam::unlimited(),
        ),
        Command::from_pattern(&tree.options.pattern),
        in_new_panel,
    )
}

impl VerbExecutor for BrowserState {
    fn execute_verb(
        &mut self,
        verb: &Verb,
        user_invocation: Option<&VerbInvocation>,
        screen: &mut Screen,
        con: &AppContext,
    ) -> Result<AppStateCmdResult, ProgramError> {
        if let Some(err) = user_invocation.and_then(|invocation| verb.match_error(invocation)) {
            return Ok(AppStateCmdResult::DisplayError(err));
        }
        let page_height = BrowserState::page_height(screen);
        Ok(match &verb.execution {
            VerbExecution::Internal{ internal, bang } => {
                use Internal::*;
                let bang = user_invocation.map(|inv| inv.bang).unwrap_or(*bang);
                match internal {
                    back => AppStateCmdResult::PopState,
                    focus => {
                        let tree = self.displayed_tree_mut();
                        let line = &tree.selected_line();
                        let mut path = line.target();
                        if !path.is_dir() {
                            path = path.parent().unwrap().to_path_buf();
                        }
                        focus_path(path, screen, tree, bang)
                    }
                    focus_root => {
                        focus_path(PathBuf::from("/"), screen, self.displayed_tree(), bang)
                    }
                    up_tree => match self.displayed_tree().root().parent() {
                        Some(path) => {
                            focus_path(path.to_path_buf(), screen, self.displayed_tree(), bang)
                        }
                        None => AppStateCmdResult::DisplayError("no parent found".to_string()),
                    },
                    focus_user_home => match UserDirs::new() {
                        Some(ud) => {
                            focus_path(ud.home_dir().to_path_buf(), screen, self.displayed_tree(), bang)
                        }
                        None => AppStateCmdResult::DisplayError("no user home directory found".to_string()),
                    },
                    help => {
                        AppStateCmdResult::NewState {
                            state: Box::new(HelpState::new(screen, con)),
                            cmd: Command::new(),
                            in_new_panel: bang,
                        }
                    }
                    open_stay => self.open_selection_stay_in_broot(screen, con, bang)?,
                    open_leave => self.open_selection_quit_broot(con)?,
                    line_down => {
                        self.displayed_tree_mut().move_selection(1, page_height);
                        AppStateCmdResult::Keep
                    }
                    line_up => {
                        self.displayed_tree_mut().move_selection(-1, page_height);
                        AppStateCmdResult::Keep
                    }
                    page_down => {
                        let tree = self.displayed_tree_mut();
                        if page_height < tree.lines.len() as i32 {
                            tree.try_scroll(page_height, page_height);
                        }
                        AppStateCmdResult::Keep
                    }
                    page_up => {
                        let tree = self.displayed_tree_mut();
                        if page_height < tree.lines.len() as i32 {
                            tree.try_scroll(-page_height, page_height);
                        }
                        AppStateCmdResult::Keep
                    }
                    parent => self.go_to_parent(screen, bang),
                    print_path => {
                        print::print_path(&self.displayed_tree().selected_line().target(), con)?
                    }
                    print_relative_path => {
                        print::print_relative_path(&self.displayed_tree().selected_line().target(), con)?
                    }
                    print_tree => {
                        print::print_tree(&self.displayed_tree(), screen, con)?
                    }
                    refresh => AppStateCmdResult::RefreshState { clear_cache: true },
                    select_first => {
                        self.displayed_tree_mut().try_select_first();
                        AppStateCmdResult::Keep
                    }
                    select_last => {
                        self.displayed_tree_mut().try_select_last();
                        AppStateCmdResult::Keep
                    }
                    toggle_dates => self.with_new_options(screen, &|o| o.show_dates ^= true, bang),
                    toggle_files => {
                        self.with_new_options(screen, &|o: &mut TreeOptions| o.only_folders ^= true, bang)
                    }
                    toggle_hidden => self.with_new_options(screen, &|o| o.show_hidden ^= true, bang),
                    toggle_git_ignore => {
                        self.with_new_options(screen, &|o| o.respect_git_ignore ^= true, bang)
                    }
                    toggle_git_file_info => {
                        self.with_new_options(screen, &|o| o.show_git_file_info ^= true, bang)
                    }
                    toggle_git_status => {
                        self.with_new_options(screen, &|o| o.filter_by_git_status ^= true, bang)
                    }
                    toggle_perm => self.with_new_options(screen, &|o| o.show_permissions ^= true, bang),
                    toggle_sizes => self.with_new_options(screen, &|o| o.show_sizes ^= true, bang),
                    toggle_trim_root => self.with_new_options(screen, &|o| o.trim_root ^= true, bang),
                    total_search => {
                        if let Some(tree) = &self.filtered_tree {
                            if tree.total_search {
                                AppStateCmdResult::DisplayError(
                                    "search was already total - all children have been rated".to_owned(),
                                )
                            } else {
                                self.pending_pattern = tree.options.pattern.clone();
                                self.total_search_required = true;
                                AppStateCmdResult::Keep
                            }
                        } else {
                            AppStateCmdResult::DisplayError(
                                "this verb can be used only after a search".to_owned(),
                            )
                        }
                    }
                    quit => AppStateCmdResult::Quit,
                }
            },
            VerbExecution::External(external) => external.to_cmd_result(
                &self.displayed_tree().selected_line().path.clone(),
                if let Some(inv) = &user_invocation {
                    &inv.args
                } else {
                    &None
                },
                con,
            )?,
        })
    }
}
