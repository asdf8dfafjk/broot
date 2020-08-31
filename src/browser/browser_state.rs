use {
    crate::{
        app::*,
        command::{Command, TriggerType},
        display::{DisplayableTree, Screen, W},
        errors::{ProgramError, TreeBuildError},
        flag::Flag,
        git,
        launchable::Launchable,
        pattern::*,
        path,
        path_anchor::PathAnchor,
        print,
        skin::PanelSkin,
        task_sync::Dam,
        tree::*,
        tree_build::TreeBuilder,
        verb::*,
    },
    open,
    std::{
        fs::OpenOptions,
        io::Write,
        path::{Path, PathBuf},
    },
    termimad::Area,
};

/// An application state dedicated to displaying a tree.
/// It's the first and main screen of broot.
pub struct BrowserState {
    pub tree: Tree,
    pub filtered_tree: Option<Tree>,
    pub pending_pattern: InputPattern, // a pattern (or not) which has not yet be applied
    pub total_search_required: bool, // whether the pending pattern should be in total search mode
}

impl BrowserState {

    /// build a new tree state if there's no error and there's no cancellation.
    ///
    /// In case of cancellation return `Ok(None)`. If dam is `unlimited()` then
    /// this can't be returned.
    pub fn new(
        path: PathBuf,
        mut options: TreeOptions,
        screen: &Screen,
        con: &AppContext,
        dam: &Dam,
    ) -> Result<Option<BrowserState>, TreeBuildError> {
        let pending_pattern = options.pattern.take();
        let builder = TreeBuilder::from(
            path,
            options,
            BrowserState::page_height(screen) as usize,
            con,
        )?;
        Ok(builder.build(false, dam).map(move |tree| BrowserState {
            tree,
            filtered_tree: None,
            pending_pattern,
            total_search_required: false,
        }))
    }

    pub fn with_new_options(
        &self,
        screen: &Screen,
        change_options: &dyn Fn(&mut TreeOptions),
        in_new_panel: bool,
        con: &AppContext,
    ) -> AppStateCmdResult {
        let tree = self.displayed_tree();
        let mut options = tree.options.clone();
        change_options(&mut options);
        AppStateCmdResult::from_optional_state(
            BrowserState::new(tree.root().clone(), options, screen, con, &Dam::unlimited()),
            in_new_panel,
        )
    }

    pub fn root(&self) -> &Path {
        self.tree.root()
    }

    pub fn page_height(screen: &Screen) -> i32 {
        i32::from(screen.height) - 2
    }

    /// return a reference to the currently displayed tree, which
    /// is the filtered tree if there's one, the base tree if not.
    pub fn displayed_tree(&self) -> &Tree {
        self.filtered_tree.as_ref().unwrap_or(&self.tree)
    }

    /// return a mutable reference to the currently displayed tree, which
    /// is the filtered tree if there's one, the base tree if not.
    pub fn displayed_tree_mut(&mut self) -> &mut Tree {
        self.filtered_tree.as_mut().unwrap_or(&mut self.tree)
    }

    pub fn open_selection_stay_in_broot(
        &mut self,
        screen: &mut Screen,
        con: &AppContext,
        in_new_panel: bool,
        keep_pattern: bool,
    ) -> Result<AppStateCmdResult, ProgramError> {
        let tree = self.displayed_tree();
        let line = tree.selected_line();
        match &line.line_type {
            TreeLineType::File => match open::that(&line.path) {
                Ok(exit_status) => {
                    info!("open returned with exit_status {:?}", exit_status);
                    Ok(AppStateCmdResult::Keep)
                }
                Err(e) => Ok(AppStateCmdResult::DisplayError(format!("{:?}", e))),
            },
            TreeLineType::Dir | TreeLineType::SymLinkToDir(_) => {
                let mut target = line.target();
                if tree.selection == 0 {
                    // opening the root would be going to where we already are.
                    // We go up one level instead
                    if let Some(parent) = target.parent() {
                        target = PathBuf::from(parent);
                    }
                }
                let dam = Dam::unlimited();
                Ok(AppStateCmdResult::from_optional_state(
                    BrowserState::new(
                        target,
                        if keep_pattern {
                            tree.options.clone()
                        } else {
                            tree.options.without_pattern()
                        },
                        screen,
                        con,
                        &dam,
                    ),
                    in_new_panel,
                ))
            }
            TreeLineType::SymLinkToFile(target) => {
                let path = PathBuf::from(target);
                open::that(&path)?;
                Ok(AppStateCmdResult::Keep)
            }
            _ => {
                unreachable!();
            }
        }
    }

    pub fn open_selection_quit_broot(
        &mut self,
        w: &mut W,
        con: &AppContext,
    ) -> Result<AppStateCmdResult, ProgramError> {
        let tree = self.displayed_tree();
        let line = tree.selected_line();
        match &line.line_type {
            TreeLineType::File => make_opener(line.path.clone(), line.is_exe(), con),
            TreeLineType::Dir | TreeLineType::SymLinkToDir(_) => {
                Ok(if con.launch_args.cmd_export_path.is_some() {
                    CD.to_cmd_result(w, line.as_selection(), &None, &None, con)?
                } else {
                    AppStateCmdResult::DisplayError(
                        "This feature needs broot to be launched with the `br` script".to_owned(),
                    )
                })
            }
            TreeLineType::SymLinkToFile(target) => {
                make_opener(
                    PathBuf::from(target),
                    line.is_exe(), // today this always return false
                    con,
                )
            }
            _ => {
                unreachable!();
            }
        }
    }

    pub fn go_to_parent(
        &mut self,
        screen: &mut Screen,
        con: &AppContext,
        in_new_panel: bool,
    ) -> AppStateCmdResult {
        match &self.displayed_tree().selected_line().path.parent() {
            Some(path) => AppStateCmdResult::from_optional_state(
                BrowserState::new(
                    path.to_path_buf(),
                    self.displayed_tree().options.without_pattern(),
                    screen,
                    con,
                    &Dam::unlimited(),
                ),
                in_new_panel,
            ),
            None => AppStateCmdResult::DisplayError("no parent found".to_string()),
        }
    }

}

/// build a AppStateCmdResult with a launchable which will be used to
///  1/ quit broot
///  2/ open the relevant file the best possible way
fn make_opener(
    path: PathBuf,
    is_exe: bool,
    con: &AppContext,
) -> Result<AppStateCmdResult, ProgramError> {
    Ok(if is_exe {
        let path = path.to_string_lossy().to_string();
        if let Some(export_path) = &con.launch_args.cmd_export_path {
            // broot was launched as br, we can launch the executable from the shell
            let f = OpenOptions::new().append(true).open(export_path)?;
            writeln!(&f, "{}", path)?;
            AppStateCmdResult::Quit
        } else {
            AppStateCmdResult::from(Launchable::program(
                vec![path],
                None, // we don't set the working directory
            )?)
        }
    } else {
        AppStateCmdResult::from(Launchable::opener(path))
    })
}

impl AppState for BrowserState {

    fn get_pending_task(&self) -> Option<&'static str> {
        if self.pending_pattern.is_some() {
            Some("searching")
        } else if self.displayed_tree().has_dir_missing_sum() {
            Some("computing stats")
        } else if self.displayed_tree().is_missing_git_status_computation() {
            Some("computing git status")
        } else {
            None
        }
    }


    fn selected_path(&self) -> &Path {
        &self.displayed_tree().selected_line().path
    }


    fn selection(&self) -> Selection<'_> {
        self.displayed_tree().selected_line().as_selection()
    }

    fn clear_pending(&mut self) {
        self.pending_pattern = InputPattern::none();
    }

    fn on_click(
        &mut self,
        _x: u16,
        y: u16,
        _screen: &mut Screen,
        _con: &AppContext,
    ) -> Result<AppStateCmdResult, ProgramError> {
        self.displayed_tree_mut().try_select_y(y as i32);
        Ok(AppStateCmdResult::Keep)
    }

    fn on_double_click(
        &mut self,
        _x: u16,
        y: u16,
        screen: &mut Screen,
        con: &AppContext,
    ) -> Result<AppStateCmdResult, ProgramError> {
        if self.displayed_tree().selection == y as usize {
            self.open_selection_stay_in_broot(screen, con, false, false)
        } else {
            // A double click always come after a simple click at
            // same position. If it's not the selected line, it means
            // the click wasn't on a selectable/openable tree line
            Ok(AppStateCmdResult::Keep)
        }
    }

    fn on_pattern(
        &mut self,
        pat: InputPattern,
        _con: &AppContext,
    ) -> Result<AppStateCmdResult, ProgramError> {
        if pat.is_none() {
            self.filtered_tree = None;
        }
        self.pending_pattern = pat;
        Ok(AppStateCmdResult::Keep)
    }

    fn on_internal(
        &mut self,
        w: &mut W,
        internal_exec: &InternalExecution,
        input_invocation: Option<&VerbInvocation>,
        trigger_type: TriggerType,
        cc: &CmdContext,
        screen: &mut Screen,
    ) -> Result<AppStateCmdResult, ProgramError> {
        let con = &cc.con;
        let page_height = BrowserState::page_height(screen);
        let bang = input_invocation
            .map(|inv| inv.bang)
            .unwrap_or(internal_exec.bang);
        Ok(match internal_exec.internal {
            Internal::back => {
                if let Some(filtered_tree) = &self.filtered_tree {
                    let filtered_selection = &filtered_tree.selected_line().path;
                    self.tree.try_select_path(filtered_selection);
                    self.filtered_tree = None;
                    AppStateCmdResult::Keep
                } else if self.tree.selection > 0 {
                    self.tree.selection = 0;
                    AppStateCmdResult::Keep
                } else {
                    AppStateCmdResult::PopState
                }
            }
            Internal::copy_path => {
                let path = &self.displayed_tree().selected_line().target();
                cli_clipboard::set_contents( path.to_string_lossy().into_owned() )
					.map_err( |_| ProgramError::ClipboardError )?
				;

				AppStateCmdResult::Keep
            }
            Internal::focus => internal_focus::on_internal(
                internal_exec,
                input_invocation,
                trigger_type,
                self.selected_path(),
                screen,
                con,
                self.displayed_tree().options.clone(),
            ),
            Internal::up_tree => match self.displayed_tree().root().parent() {
                Some(path) => internal_focus::on_path(
                    path.to_path_buf(),
                    screen,
                    self.displayed_tree().options.clone(),
                    bang,
                    con,
                ),
                None => AppStateCmdResult::DisplayError("no parent found".to_string()),
            },
            Internal::open_stay => self.open_selection_stay_in_broot(screen, con, bang, false)?,
            Internal::open_stay_filter => self.open_selection_stay_in_broot(screen, con, bang, true)?,
            Internal::open_leave => self.open_selection_quit_broot(w, con)?,
            Internal::line_down => {
                self.displayed_tree_mut().move_selection(1, page_height);
                AppStateCmdResult::Keep
            }
            Internal::line_up => {
                self.displayed_tree_mut().move_selection(-1, page_height);
                AppStateCmdResult::Keep
            }
            Internal::previous_match => {
                self.displayed_tree_mut().try_select_previous_match();
                AppStateCmdResult::Keep
            }
            Internal::next_match => {
                self.displayed_tree_mut().try_select_next_match();
                AppStateCmdResult::Keep
            }
            Internal::page_down => {
                let tree = self.displayed_tree_mut();
                if page_height < tree.lines.len() as i32 {
                    tree.try_scroll(page_height, page_height);
                }
                AppStateCmdResult::Keep
            }
            Internal::page_up => {
                let tree = self.displayed_tree_mut();
                if page_height < tree.lines.len() as i32 {
                    tree.try_scroll(-page_height, page_height);
                }
                AppStateCmdResult::Keep
            }
            Internal::panel_left => {
                if cc.areas.is_first() {
                    if cc.preview.is_some() && cc.areas.nb_pos == 2 {
                        AppStateCmdResult::ClosePanel {
                            validate_purpose: false,
                            id: cc.preview,
                        }
                    } else {
                        // we ask for the creation of a panel to the left
                        internal_focus::new_panel_on_path(
                            self.selected_path().to_path_buf(),
                            screen,
                            self.displayed_tree().options.clone(),
                            PanelPurpose::None,
                            con,
                            HDir::Left,
                        )
                    }
                } else {
                    // we ask the app to focus the panel to the left
                    AppStateCmdResult::HandleInApp(Internal::panel_left)
                }
            }
            Internal::panel_right => {
                if cc.areas.is_last() {
                    let purpose = if self.selected_path().is_file() && cc.preview.is_none() {
                        PanelPurpose::Preview
                    } else {
                        PanelPurpose::None
                    };
                    // we ask for the creation of a panel to the right
                    internal_focus::new_panel_on_path(
                        self.selected_path().to_path_buf(),
                        screen,
                        self.displayed_tree().options.clone(),
                        purpose,
                        con,
                        HDir::Right,
                    )
                } else {
                    // we ask the app to focus the panel to the left
                    AppStateCmdResult::HandleInApp(Internal::panel_right)
                }
            }
            Internal::parent => self.go_to_parent(screen, con, bang),
            Internal::print_path => {
                let path = &self.displayed_tree().selected_line().target();
                print::print_path(path, con)?
            }
            Internal::print_relative_path => {
                let path = &self.displayed_tree().selected_line().target();
                print::print_relative_path(path, con)?
            }
            Internal::print_tree => {
                print::print_tree(&self.displayed_tree(), screen, &cc.panel_skin, con)?
            }
            Internal::select_first => {
                self.displayed_tree_mut().try_select_first();
                AppStateCmdResult::Keep
            }
            Internal::select_last => {
                let page_height = BrowserState::page_height(screen);
                self.displayed_tree_mut().try_select_last(page_height);
                AppStateCmdResult::Keep
            }
            Internal::start_end_panel => {
                if cc.panel_purpose.is_arg_edition() {
                    debug!("start_end understood as end");
                    AppStateCmdResult::ClosePanel {
                        validate_purpose: true,
                        id: None,
                    }
                } else {
                    debug!("start_end understood as start");
                    let tree_options = self.displayed_tree().options.clone();
                    if let Some(input_invocation) = input_invocation {
                        // we'll go for input arg editing
                        let path = if let Some(input_arg) = &input_invocation.args {
                            path::path_from(self.root(), PathAnchor::Unspecified, input_arg)
                        } else {
                            self.root().to_path_buf()
                        };
                        let arg_type = SelectionType::Any; // We might do better later
                        let purpose = PanelPurpose::ArgEdition { arg_type };
                        internal_focus::new_panel_on_path(
                            path, screen, tree_options, purpose, con, HDir::Right,
                        )
                    } else {
                        // we just open a new panel on the selected path,
                        // without purpose
                        internal_focus::new_panel_on_path(
                            self.selected_path().to_path_buf(),
                            screen,
                            tree_options,
                            PanelPurpose::None,
                            con,
                            HDir::Right,
                        )
                    }
                }
            }
            Internal::sort_by_count => {
                self.with_new_options(
                    screen, &|o| {
                        if o.sort == Sort::Count {
                            o.sort = Sort::None;
                            o.show_counts = false;
                        } else {
                            o.sort = Sort::Count;
                            o.show_counts = true;
                        }
                    },
                    bang,
                    con,
                )
            }
            Internal::sort_by_date => {
                self.with_new_options(
                    screen, &|o| {
                        if o.sort == Sort::Date {
                            o.sort = Sort::None;
                            o.show_dates = false;
                        } else {
                            o.sort = Sort::Date;
                            o.show_dates = true;
                        }
                    },
                    bang,
                    con,
                )
            }
            Internal::sort_by_size => {
                self.with_new_options(
                    screen, &|o| {
                        if o.sort == Sort::Size {
                            o.sort = Sort::None;
                            o.show_sizes = false;
                        } else {
                            o.sort = Sort::Size;
                            o.show_sizes = true;
                        }
                    },
                    bang,
                    con,
                )
            }
            Internal::no_sort => {
                self.with_new_options(screen, &|o| o.sort = Sort::None, bang, con)
            }
            Internal::toggle_counts => {
                self.with_new_options(screen, &|o| o.show_counts ^= true, bang, con)
            }
            Internal::toggle_dates => {
                self.with_new_options(screen, &|o| o.show_dates ^= true, bang, con)
            }
            Internal::toggle_files => {
                self.with_new_options(screen, &|o: &mut TreeOptions| o.only_folders ^= true, bang, con)
            }
            Internal::toggle_hidden => {
                self.with_new_options(screen, &|o| o.show_hidden ^= true, bang, con)
            }
            Internal::toggle_git_ignore => {
                self.with_new_options(screen, &|o| o.respect_git_ignore ^= true, bang, con)
            }
            Internal::toggle_git_file_info => {
                self.with_new_options(screen, &|o| o.show_git_file_info ^= true, bang, con)
            }
            Internal::toggle_git_status => {
                self.with_new_options(
                    screen, &|o| {
                        if o.filter_by_git_status {
                            o.filter_by_git_status = false;
                        } else {
                            o.filter_by_git_status = true;
                            o.show_hidden = true;
                        }
                    }, bang, con
                )
            }
            Internal::toggle_perm => {
                self.with_new_options(screen, &|o| o.show_permissions ^= true, bang, con)
            }
            Internal::toggle_sizes => {
                self.with_new_options(screen, &|o| o.show_sizes ^= true, bang, con)
            }
            Internal::toggle_trim_root => {
                self.with_new_options(screen, &|o| o.trim_root ^= true, bang, con)
            }
            Internal::total_search => {
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
            Internal::quit => AppStateCmdResult::Quit,
            _ => self.on_internal_generic(
                w,
                internal_exec,
                input_invocation,
                trigger_type,
                cc,
                screen,
            )?,
        })
    }

    fn no_verb_status(
        &self,
        has_previous_state: bool,
        con: &AppContext,
    ) -> Status {
        let mut ssb = con.standard_status.builder(
            AppStateType::Tree,
            self.selection(),
        );
        ssb.has_previous_state = has_previous_state;
        ssb.is_filtered = self.filtered_tree.is_some();
        ssb.has_removed_pattern = false;
        ssb.on_tree_root = self.displayed_tree().selection == 0;
        ssb.status()
    }

    /// do some work, totally or partially, if there's some to do.
    /// Stop as soon as the dam asks for interruption
    fn do_pending_task(
        &mut self,
        screen: &mut Screen,
        con: &AppContext,
        dam: &mut Dam,
    ) {
        if self.pending_pattern.is_some() {
            let pattern_str = self.pending_pattern.raw.clone();
            let mut options = self.tree.options.clone();
            options.pattern = self.pending_pattern.take();
            let root = self.tree.root().clone();
            let page_height = BrowserState::page_height(screen) as usize;
            let builder = match TreeBuilder::from(root, options, page_height, con) {
                Ok(builder) => builder,
                Err(e) => {
                    warn!("Error while preparing tree builder: {:?}", e);
                    return;
                }
            };
            let mut filtered_tree = time!(
                Info,
                "tree filtering",
                &pattern_str,
                builder.build(self.total_search_required, dam),
            ); // can be None if a cancellation was required
            self.total_search_required = false;
            if let Some(ref mut ft) = filtered_tree {
                ft.try_select_best_match();
                ft.make_selection_visible(BrowserState::page_height(screen));
                self.filtered_tree = filtered_tree;
            }
        } else if self.displayed_tree().is_missing_git_status_computation() {
            let root_path = self.displayed_tree().root();
            let git_status = git::get_tree_status(root_path, dam);
            self.displayed_tree_mut().git_status = git_status;
        } else {
            self.displayed_tree_mut().fetch_some_missing_dir_sum(dam);
        }
    }

    fn display(
        &mut self,
        w: &mut W,
        _screen: &Screen,
        area: Area,
        panel_skin: &PanelSkin,
        con: &AppContext,
    ) -> Result<(), ProgramError> {
        let dp = DisplayableTree {
            tree: &self.displayed_tree(),
            skin: &panel_skin.styles,
            cols: &con.cols,
            show_selection_mark: con.show_selection_mark,
            ext_colors: &con.ext_colors,
            area,
            in_app: true,
        };
        dp.write_on(w)
    }

    fn refresh(&mut self, screen: &Screen, con: &AppContext) -> Command {
        let page_height = BrowserState::page_height(screen) as usize;
        // refresh the base tree
        if let Err(e) = self.tree.refresh(page_height, con) {
            warn!("refreshing base tree failed : {:?}", e);
        }
        // refresh the filtered tree, if any
        Command::from_pattern(
            match self.filtered_tree {
                Some(ref mut tree) => {
                    if let Err(e) = tree.refresh(page_height, con) {
                        warn!("refreshing filtered tree failed : {:?}", e);
                    }
                    &tree.options.pattern
                }
                None => &self.tree.options.pattern,
            },
        )
    }

    fn get_flags(&self) -> Vec<Flag> {
        let options = &self.displayed_tree().options;
        vec![
            Flag {
                name: "h",
                value: if options.show_hidden { "y" } else { "n" },
            },
            Flag {
                name: "gi",
                value: if options.respect_git_ignore { "y" } else { "n" },
            },
        ]
    }

    fn get_starting_input(&self) -> String {
        if self.pending_pattern.is_some() {
            self.pending_pattern.raw.clone()
        } else {
            self.displayed_tree().options.pattern.raw.clone()
        }
    }
}

