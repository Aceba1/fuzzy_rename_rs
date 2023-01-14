use std::{fs, path::PathBuf};

use egui::*;
use egui_extras::{Column, TableBuilder};
use rfd::FileDialog;

use std::fs::read_dir;

const CHOICE_PREVIEW_COUNT: usize = 10;

fn remove_extension(s: &str) -> &str {
    &s[0..s.rfind('.').unwrap_or(s.len())]
}

#[derive(Clone, Default)]
struct FilePath {
    name: String,
    path: PathBuf,
}

impl TryFrom<PathBuf> for FilePath {
    type Error = ();

    fn try_from(value: PathBuf) -> Result<Self, Self::Error> {
        let filename = value
            .file_name()
            .and_then(|f| f.to_str().map(|f| f.to_owned()));
        filename
            .map(|name| Self { path: value, name })
            .ok_or(Default::default())
    }
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Eq)]
enum SearchAlgorithm {
    Jaro,
    JaroWinkler,
    Levenshtein,
    DamerauLevenshtein,
}

impl SearchAlgorithm {
    fn compare(&self, a: &str, b: &str) -> f64 {
        match self {
            SearchAlgorithm::Jaro => strsim::jaro(a, b),
            SearchAlgorithm::JaroWinkler => strsim::jaro_winkler(a, b),
            SearchAlgorithm::Levenshtein => strsim::normalized_levenshtein(a, b),
            SearchAlgorithm::DamerauLevenshtein => strsim::normalized_damerau_levenshtein(a, b),
        }
    }
}

#[derive(Default)]
struct SourceName {
    file: FilePath,
    choice_map: Vec<(usize, f32)>,
    manual_choice: Option<Option<usize>>,
}

impl TryFrom<PathBuf> for SourceName {
    type Error = ();

    fn try_from(value: PathBuf) -> Result<Self, Self::Error> {
        FilePath::try_from(value).map(|file| Self {
            file,
            ..Default::default()
        })
    }
}

impl SourceName {
    #[inline]
    fn reset_choice(&mut self) {
        self.manual_choice = None;
    }

    #[inline(always)]
    fn set_choice(&mut self, index: Option<usize>) {
        self.manual_choice = Some(index);
    }

    fn current_choice(&self) -> Option<usize> {
        match self.manual_choice {
            Some(choice) => choice,
            None => self.choice_map.first().map(|(i, _)| *i),
        }
    }

    fn current_score(&self) -> Option<f32> {
        if self.manual_choice.is_none() {
            Some(self.choice_map.first().map_or(0.0, |(_, s)| *s))
        } else {
            None
        }
    }

    fn update_choices(&mut self, choice_names: &Vec<FilePath>, algorithm: &SearchAlgorithm) {
        // let mut scores: BTreeMap<u32, (usize, f32)> = BTreeMap::new();
        let name = remove_extension(&self.file.name);
        let mut scores: [(usize, f32); CHOICE_PREVIEW_COUNT] = [(0, -1.0); 10];

        for (index, choice) in choice_names.iter().enumerate() {
            let score = algorithm.compare(name, remove_extension(&choice.name)) as f32;

            let mut lowest: f32 = 2.0; // Or infinity
            let mut replace: usize = 0;
            for i in 0usize..CHOICE_PREVIEW_COUNT {
                let (_, i_score) = scores[i];
                if i_score < score && i_score < lowest {
                    lowest = i_score;
                    replace = i;
                }
            }
            if lowest != 2.0 {
                scores[replace] = (index, score);
            }
        }

        self.choice_map =
            Vec::from(&scores[0..scores.iter().position(|(_, s)| -1.0 == *s).unwrap_or(10)]);
        self.choice_map.sort_by(|a, b| b.1.total_cmp(&a.1));
        // self.choice_map = scores.into_values().rev().take(10).collect();
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
struct FuzzySearch {
    #[serde(skip)]
    source_names: Vec<SourceName>,
    #[serde(skip)]
    choice_names: Vec<FilePath>,

    algorithm: SearchAlgorithm,
}

impl Default for FuzzySearch {
    fn default() -> Self {
        Self {
            source_names: vec![],
            choice_names: vec![],
            algorithm: SearchAlgorithm::Jaro,
        }
    }
}

impl FuzzySearch {
    fn add_source(&mut self, path: PathBuf) {
        if let Ok(mut source) = SourceName::try_from(path) {
            source.update_choices(&self.choice_names, &self.algorithm);
            self.source_names.push(source);
        }
    }

    fn add_choice(&mut self, path: PathBuf) {
        if let Ok(choice) = FilePath::try_from(path) {
            self.choice_names.push(choice);
        }
    }

    fn update_all(&mut self) {
        self.source_names
            .sort_unstable_by_key(|v| v.file.name.clone());
        for source in self.source_names.iter_mut() {
            source.update_choices(&self.choice_names, &self.algorithm);
        }
    }

    fn remove_source(&mut self, index: usize) {
        self.source_names.remove(index);
    }

    // fn remove_choice(&mut self, index: usize) {
    //     self.choice_names.swap_remove(index);
    // }
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Eq)]
enum WindowTheme {
    Dark,
    Light,
}

#[derive(serde::Deserialize, serde::Serialize, PartialEq, Eq)]
enum SideToUse {
    Choices,
    Sources,
}

enum AppStatus {
    None,
    Info(String),
    Notice(String),
    // Progress(String, f32),
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct MainApp {
    sources_path: String,
    choices_path: String,
    renames_path: String,

    keep_extension: bool,
    side_to_copy: SideToUse,
    copy_failed_sources: bool,

    window_theme: WindowTheme,

    threshold: f32,

    search: FuzzySearch,

    #[serde(skip)]
    status: AppStatus,
}

impl Default for MainApp {
    fn default() -> Self {
        Self {
            sources_path: "".to_owned(),
            choices_path: "".to_owned(),
            renames_path: "".to_owned(),
            keep_extension: false,
            side_to_copy: SideToUse::Choices,
            copy_failed_sources: true,
            window_theme: WindowTheme::Light,
            threshold: 0.7,
            search: FuzzySearch::default(),
            status: AppStatus::None,
        }
    }
}

impl MainApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let main_app: Self = if let Some(storage) = cc.storage {
            // Loads the previous state
            eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default()
        } else {
            // Loads with default values
            Default::default()
        };

        Self::set_window_theme(&cc.egui_ctx, &main_app.window_theme);

        main_app
    }

    fn set_window_theme(ctx: &Context, theme: &WindowTheme) {
        ctx.set_visuals(match theme {
            WindowTheme::Dark => Visuals::dark(),
            WindowTheme::Light => Visuals::light(),
        });
    }

    // Rename choice
    //   A_game.zip
    // > A.png <
    // > A_game.zip.png <

    // Rename source
    // > B.png <
    //   B_game.zip
    // > B_game.zip.png <

    fn iter_renames(&self, include_failed: bool) -> Vec<(&PathBuf, String)> {
        return self
            .search
            .source_names
            .iter()
            .filter_map(|source| {
                let current_choice = source.current_choice();
                let below_threshold = source.current_score().map_or(false, |s| s < self.threshold);

                let choice = current_choice.and_then(|c| self.search.choice_names.get(c));
                if let Some(choice) = choice.filter(|_| !below_threshold) {
                    let rename = self.rename(&source.file.name, &choice.name);
                    let path = match self.side_to_copy {
                        SideToUse::Choices => &choice.path,
                        SideToUse::Sources => &source.file.path,
                    };
                    Some((path, rename))
                } else if include_failed && self.side_to_copy == SideToUse::Sources {
                    Some((&source.file.path, source.file.name.clone()))
                } else {
                    None
                }
            })
            .collect();
    }

    fn rename(&self, source: &str, choice: &str) -> String {
        let (original, reference) = match self.side_to_copy {
            SideToUse::Choices => (choice, source),
            SideToUse::Sources => (source, choice),
        };

        let extension = original.rsplit_once('.').map_or("", |(_, s)| s);
        let body = if self.keep_extension {
            reference
        } else {
            remove_extension(reference)
        };
        format!("{body}.{extension}")
    }
}

impl eframe::App for MainApp {
    /// Called by the frame work to save state before shutdown.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, self);
    }

    /// Called each time the UI needs repainting, which may be many times per second.
    /// Put your widgets into a `SidePanel`, `TopPanel`, `CentralPanel`, `Window` or `Area`.
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        TopBottomPanel::top("top_panel").show(ctx, |ui| {
            // Files bar
            menu::bar(ui, |ui| {
                // Sources
                ui.menu_button("Sources", |ui| {
                    ui.weak("Base names to match to");
                    ui.weak("All files will be used");
                    ui.separator();

                    // We can't import from folders on the web
                    ui.add_enabled_ui(cfg!(not(target_arch = "wasm32")), |ui| {
                        if ui.button("Import folder").clicked() {
                            let folder = FileDialog::new()
                                .set_title("Choose a folder with source files")
                                .set_directory(&self.sources_path)
                                .pick_folder();

                            if let Some(folder) = folder {
                                self.sources_path = folder.to_str().unwrap().to_owned();

                                if let Ok(read_dir) = read_dir(folder) {
                                    let mut count = 0usize;
                                    for item in read_dir.filter_map(|i| i.ok()) {
                                        if item.file_type().map_or(false, |f| f.is_file()) {
                                            self.search.add_source(item.path());
                                            count += 1;
                                        }
                                    }
                                    self.status = AppStatus::Info(format!("Added {count} source(s)"));
                                }
                            }
                        }
                    });

                    if ui.button("Import files").clicked() {
                        let files = FileDialog::new()
                            .set_title("Choose source files")
                            .set_directory(&self.sources_path)
                            .pick_files();

                        if let Some(files) = files {
                            if let Some(file) = files.first() {
                                self.sources_path =
                                    file.parent().unwrap().to_str().unwrap().to_owned();
                            }

                            let count = files.len();
                            for item in files {
                                self.search.add_source(item);
                            }
                            self.status = AppStatus::Info(format!("Added {count} source(s)"));
                        }
                    }

                    ui.separator();

                    ui.menu_button("Clear all sources", |ui| {
                        ui.label("Are you sure?");
                        if ui.button("Yes").clicked() {
                            self.search.source_names.clear();
                            self.status = AppStatus::Info("Cleared all sources".to_owned());
                        }
                    })
                });

                // Choices
                ui.menu_button("Choices", |ui| {
                    ui.weak("References to match with");
                    ui.weak("Matched files will be used");
                    ui.separator();

                    // We can't import from folders on the web
                    ui.add_enabled_ui(cfg!(not(target_arch = "wasm32")), |ui| {
                        if ui.button("Import folder").clicked() {
                            let folder = FileDialog::new()
                                .set_title("Choose a folder with reference files")
                                .set_directory(&self.choices_path)
                                .pick_folder();

                            if let Some(folder) = folder {
                                self.choices_path = folder.to_str().unwrap().to_owned();

                                if let Ok(read_dir) = read_dir(folder) {
                                    let mut count = 0usize;
                                    for item in read_dir.filter_map(|i| i.ok()) {
                                        if item.file_type().map_or(false, |f| f.is_file()) {
                                            self.search.add_choice(item.path());
                                            count += 1;
                                        }
                                    }
                                    if count > 0 {
                                        self.search.update_all();
                                    }
                                    self.status = AppStatus::Info(format!("Added {count} reference(s)"));
                                }
                            }
                        }
                    });

                    if ui.button("Import files").clicked() {
                        let files = FileDialog::new()
                            .set_title("Choose reference files")
                            .set_directory(&self.choices_path)
                            .pick_files();

                        if let Some(files) = files {
                            if !files.is_empty() {
                                if let Some(file) = files.first() {
                                    self.choices_path =
                                        file.parent().unwrap().to_str().unwrap().to_owned();
                                }

                                let count = files.len();
                                for item in files {
                                    self.search.add_choice(item);
                                }
                                self.search.update_all();
                                self.status = AppStatus::Info(format!("Added {count} reference(s)"));
                            }
                        }
                    }

                    ui.separator();

                    ui.add_enabled_ui(false, |ui| {
                        if ui.button("Manage references...").clicked() {
                            // TODO: Open Window dialog with checked list
                        }
                    });

                    ui.menu_button("Clear all references", |ui| {
                        ui.label("Are you sure?");
                        if ui.button("Yes").clicked() {
                            self.search.choice_names.clear();
                            self.search.update_all();
                            self.status = AppStatus::Info("Cleared all references".to_owned());
                        }
                    })
                });

                // Renames
                ui.menu_button("Output", |ui| {
                    ui.weak("Results of fuzzy rename");
                    ui.weak("Files are copied to output");

                    ui.separator();

                    ui.toggle_value(&mut self.keep_extension, "Keep extensions");

                    ui.weak("Files to copy:");
                    ui.radio_value(&mut self.side_to_copy, SideToUse::Choices, "Rename Choices");
                    ui.radio_value(&mut self.side_to_copy, SideToUse::Sources, "Rename Sources");

                    ui.separator();

                    // We can't really do anything with folders on the web
                    ui.add_enabled_ui(cfg!(not(target_arch = "wasm32")), |ui| {
                        if ui.button("Copy results to folder").clicked() {
                            let folder = FileDialog::new()
                                .set_title("Choose a folder to copy renamed files to")
                                .set_directory(&self.renames_path)
                                .pick_folder();

                            if let Some(folder) = folder {
                                self.renames_path = folder.to_str().unwrap().to_owned();

                                let mut copy_count = 0usize;
                                let mut replace_count = 0usize;
                                let mut failed_count = 0usize;

                                for (file_origin, new_name) in self.iter_renames(self.copy_failed_sources) {
                                    let destination = folder.join(new_name);

                                    match destination.try_exists().and_then(|overwrite| {
                                        fs::copy(file_origin, &destination).map(|_| overwrite)
                                    }) {
                                        Ok(true) => {
                                            replace_count += 1;
                                            copy_count += 1;
                                        }
                                        Ok(false) => {
                                            copy_count += 1;
                                        }
                                        Err(error) => {
                                            eprintln!("Could not copy file: {} ({:?} -> {:?})", error, file_origin, destination.to_str());
                                            failed_count += 1;
                                        }
                                    }
                                }

                                let mut results: Vec<String> = Vec::with_capacity(3);
                                if copy_count > 0 {
                                    results.push(format!("{copy_count} Copied"));
                                }
                                if replace_count > 0 {
                                    results.push(format!("{replace_count} Replaced"));
                                }
                                if failed_count > 0 {
                                    results.push(format!("{failed_count} Failed"));
                                }
                                self.status = AppStatus::Notice(results.join(" | "));
                            }
                        }

                        if self.side_to_copy == SideToUse::Sources {
                            ui.toggle_value(
                                &mut self.copy_failed_sources,
                                "Include missing results",
                            );

                            ui.separator();

                            ui.menu_button("Directly rename files", |ui| {
                                ui.label("Are you sure?");
                                if ui.button("Yes").clicked() {
                                    let mut rename_count = 0usize;
                                    let mut replace_count = 0usize;
                                    let mut failed_count = 0usize;

                                    for (file_origin, new_name) in self.iter_renames(false) {
                                        if let Some(destination) = file_origin.parent().map(|p| p.join(new_name)) {
                                            match destination.try_exists().and_then(|overwrite| {
                                                fs::rename(file_origin, &destination).map(|_| overwrite)
                                            }) {
                                                Ok(true) => {
                                                    replace_count += 1;
                                                    rename_count += 1;
                                                }
                                                Ok(false) => {
                                                    rename_count += 1;
                                                }
                                                Err(error) => {
                                                    eprintln!("Could not rename file: {} ({:?} -> {:?})", error, file_origin, destination.to_str());
                                                    failed_count += 1;
                                                }
                                            }
                                        } else {
                                            eprintln!("Could not rename file: Malformed parent in filepath ({:?})", file_origin);
                                            failed_count += 1;
                                        }
                                    }

                                    let mut results: Vec<String> = Vec::with_capacity(3);
                                    if rename_count > 0 {
                                        results.push(format!("{rename_count} Renamed"));
                                    }
                                    if replace_count > 0 {
                                        results.push(format!("{replace_count} Replaced"));
                                    }
                                    if failed_count > 0 {
                                        results.push(format!("{failed_count} Failed"));
                                    }
                                    self.status = AppStatus::Notice(results.join(" | "));
                                }
                            });
                        }
                    });
                });

                ui.separator();

                // Options
                ui.menu_button("Options", |ui| {
                    ui.horizontal(|ui| {
                        ui.add(Slider::new(&mut self.threshold, 0.0..=1.0).text("Similarity"));
                    });

                    let mut changed;
                    ui.weak("Search Algorithm:");
                    changed = ui
                        .radio_value(&mut self.search.algorithm, SearchAlgorithm::Jaro, "Jaro")
                        .changed();
                    changed = ui
                        .radio_value(
                            &mut self.search.algorithm,
                            SearchAlgorithm::JaroWinkler,
                            "Jaro Winkler",
                        )
                        .changed()
                        | changed;
                    changed = ui
                        .radio_value(
                            &mut self.search.algorithm,
                            SearchAlgorithm::Levenshtein,
                            "Levenshtein",
                        )
                        .changed()
                        | changed;
                    changed = ui
                        .radio_value(
                            &mut self.search.algorithm,
                            SearchAlgorithm::DamerauLevenshtein,
                            "Damerau Levenshtein",
                        )
                        .changed()
                        | changed;
                    if changed {
                        self.search.update_all();
                        self.status = AppStatus::Info("Updated search algorithm".to_owned());
                    }

                    ui.separator();

                    ui.weak("Window Theme:");
                    let mut changed;
                    changed = ui
                        .radio_value(&mut self.window_theme, WindowTheme::Light, "Light")
                        .changed();
                    changed = ui
                        .radio_value(&mut self.window_theme, WindowTheme::Dark, "Dark")
                        .changed()
                        | changed;
                    if changed {
                        Self::set_window_theme(&ctx, &self.window_theme)
                    }
                });

                ui.add_space(50.0);

                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // Little helper in case it's a debug build
                        warn_if_debug_build(ui);

                        match &self.status {
                            AppStatus::None => {}
                            AppStatus::Info(message) => {
                                ui.weak(message);
                            }
                            AppStatus::Notice(message) => {
                                ui.strong(message);
                            }
                            // AppStatus::Progress(message, value) => {
                            //     ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            //         let progress_bar = ProgressBar::new(*value).show_percentage();
                            //         ui.weak(message);
                            //         ui.add(progress_bar);
                            //     });
                            // }
                        }
                    });
                });
            });
        });

        // Table

        CentralPanel::default().show(ctx, |ui| {
            ui.style_mut().wrap = Some(false);

            enum ListTask {
                None,
                RemoveRow(usize)
            }

            let mut task = ListTask::None;

            TableBuilder::new(ui)
                .striped(true)
                .auto_shrink([false; 2])
                .column(
                    Column::remainder()
                        .clip(true)
                        .at_least(100.0)
                        .resizable(true),
                )
                .column(
                    Column::initial(60.0)
                        .range(35.0..=60.0)
                        .clip(true)
                        .resizable(true),
                )
                .column(
                    Column::remainder()
                        .clip(true)
                        .at_least(100.0)
                        .resizable(true),
                )
                .column(Column::remainder().clip(true).at_least(100.0))
                .header(20.0, |mut header| {
                    header.col(|ui| {
                        ui.label("Source Name");
                    });
                    header.col(|ui| {
                        ui.label("Similarity");
                    });
                    header.col(|ui| {
                        ui.label("Closest Match");
                    });
                    header.col(|ui| {
                        ui.label(format!(
                            "Renamed File ({})",
                            match self.side_to_copy {
                                SideToUse::Choices => "Choices",
                                SideToUse::Sources => "Sources",
                            }
                        ));
                    });
                })
                .body(|body| {
                    body.rows(
                        20.0,
                        self.search.source_names.len(),
                        |row_index, mut row| {
                            let item = &mut self.search.source_names[row_index];

                            // Source Name

                            let item_name = item.file.name.clone();

                            row.col(|ui| {
                                ui.label(&item_name);
                            });

                            // Similarity

                            let current_score = item.current_score();
                            let below_threshold =
                                current_score.map_or(false, |s| s < self.threshold);

                            let choice_similarity = if let Some(value) = current_score {
                                format!("{:2.0}%", 100.0 * value)
                            } else {
                                "N/A".to_owned()
                            };

                            row.col(|ui| {
                                ui.menu_button(choice_similarity, |ui| {
                                    ui.add_enabled_ui(false, |ui| {
                                        if ui.button("Pick a match...").clicked() {
                                            // TODO: Add match picker window
                                        }
                                    });

                                    ui.separator();

                                    for (c_index, choice) in
                                        item.choice_map.clone().iter().enumerate()
                                    {
                                        let mut btn = Button::new(format!(
                                            "[{:2.2}%] {}",
                                            100.0 * choice.1,
                                            remove_extension(
                                                &self.search.choice_names[choice.0].name
                                            )
                                        ));
                                        if (c_index % 2) == 1 {
                                            btn = btn.fill(ui.visuals().faint_bg_color);
                                        }

                                        if ui.add(btn).clicked() {
                                            item.set_choice(Some(choice.0));
                                        }
                                    }
                                    if ui.button("[Don't use match]").clicked() {
                                        item.set_choice(None);
                                    }

                                    ui.separator();

                                    if item.manual_choice.is_some() {
                                        if ui.button("Restore default").clicked() {
                                            item.reset_choice();
                                        }
                                    }
                                    ui.menu_button("Remove source", |ui| {
                                        ui.label("Are you sure?");
                                        if ui.button("Yes").clicked() {
                                            task = ListTask::RemoveRow(row_index);
                                        }
                                    })
                                });
                            });

                            // Closest Match

                            let choice_name = item
                                .current_choice()
                                .filter(|_| !below_threshold)
                                .and_then(|i| self.search.choice_names.get(i).map(|c| &c.name));

                            row.col(|ui| {
                                ui.label(choice_name.unwrap_or(&"".into()));
                            });

                            // Renamed File

                            row.col(|ui| {
                                ui.label(choice_name.map_or("".to_owned(), |reference| {
                                    self.rename(&item_name, &reference)
                                }));
                            });

                            // Column end

                            match task {
                                ListTask::None => {},
                                ListTask::RemoveRow(row_index) => {
                                    self.search.remove_source(row_index);
                                    self.status = AppStatus::Info("Removed 1 source".to_owned());
                                }
                            }
                        },
                    );
                });
            //});
        });
    }
}
