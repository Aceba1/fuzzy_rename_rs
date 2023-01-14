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

    // fn remove_source(&mut self, index: usize) {
    //     self.source_names.remove(index);
    // }

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
    Sources,
    Choices,
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
                let score = source.current_score();
                if current_choice.is_some() && score.map_or(true, |s| s >= self.threshold) {
                    let choice = &self.search.choice_names[current_choice.unwrap()];
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
            SideToUse::Sources => (source, choice),
            SideToUse::Choices => (choice, source),
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
                    ui.label("Base names to match to");
                    ui.label("All files will be used");
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
                                    for item in read_dir.filter_map(|i| i.ok()) {
                                        if item.file_type().map_or(false, |f| f.is_file()) {
                                            self.search.add_source(item.path());
                                        }
                                    }
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

                            for item in files {
                                self.search.add_source(item);
                            }
                        }
                    }

                    ui.separator();

                    ui.menu_button("Clear all sources", |ui| {
                        ui.label("Are you sure?");
                        if ui.button("Yes").clicked() {
                            self.search.source_names.clear();
                        }
                    })
                });

                // Choices
                ui.menu_button("Choices", |ui| {
                    ui.label("References to match with");
                    ui.label("Matched files will be used");
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
                                    let mut flag = false;
                                    for item in read_dir.filter_map(|i| i.ok()) {
                                        if item.file_type().map_or(false, |f| f.is_file()) {
                                            self.search.add_choice(item.path());
                                            flag |= true;
                                        }
                                    }
                                    if flag {
                                        self.search.update_all();
                                    }
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

                                for item in files {
                                    self.search.add_choice(item);
                                }
                                self.search.update_all();
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
                            // Delete all references
                        }
                    })
                });

                // Renames
                ui.menu_button("Output", |ui| {
                    ui.label("Results of fuzzy rename");
                    ui.label("Files are copied to output");

                    ui.separator();

                    ui.toggle_value(&mut self.keep_extension, "Keep extensions");

                    ui.label("Files to copy:");
                    ui.radio_value(&mut self.side_to_copy, SideToUse::Sources, "Rename Sources");
                    ui.radio_value(&mut self.side_to_copy, SideToUse::Choices, "Rename Choices");

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

                                for rename in self.iter_renames(self.copy_failed_sources) {
                                    fs::copy(rename.0, folder.join(rename.1))
                                        .expect("Should copy renamed file"); // TODO: Source can be deleted
                                }
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
                                    for rename in self.iter_renames(false) {
                                        fs::rename(
                                            rename.0,
                                            rename.0.parent().unwrap().join(rename.1),
                                        )
                                        .expect("Should rename source file"); // TODO: Fails on repeated run
                                    }
                                }
                            });
                        }
                    });
                });

                ui.separator();

                // Options
                ui.menu_button("Options", |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Similarity Threshold");
                        ui.add(Slider::new(&mut self.threshold, 0.0..=1.0));
                    });

                    let mut changed;
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
                    }

                    ui.separator();

                    ui.label("Window Theme:");
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

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // Little helper in case it's a debug build
                    warn_if_debug_build(ui);
                });
            });
        });

        // Table

        CentralPanel::default().show(ctx, |ui| {
            //ui.with_layout(Layout::centered_and_justified(Direction::TopDown), |ui| {

            TableBuilder::new(ui)
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
                                SideToUse::Sources => "Sources",
                                SideToUse::Choices => "Choices",
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
                                    if item.manual_choice.is_some() {
                                        if ui.button("Restore default").clicked() {
                                            item.reset_choice();
                                        }
                                    }

                                    ui.add_enabled_ui(false, |ui| {
                                        if ui.button("Pick a match...").clicked() {
                                            // TODO: Add match picker window
                                        }
                                    });

                                    ui.separator();

                                    for choice in item.choice_map.clone() {
                                        if ui
                                            .button(format!(
                                                "[{:2.2}%] {}",
                                                100.0 * choice.1,
                                                remove_extension(
                                                    &self.search.choice_names[choice.0].name
                                                )
                                            ))
                                            .clicked()
                                        {
                                            item.set_choice(Some(choice.0));
                                        }
                                    }
                                    if ui.button("[No match]").clicked() {
                                        item.set_choice(None);
                                    }
                                });
                            });

                            // Closest Match

                            let choice_name = item
                                .current_choice()
                                .filter(|_| !below_threshold)
                                .map(|i| &self.search.choice_names[i].name);

                            row.col(|ui| {
                                ui.label(choice_name.unwrap_or(&"".into()));
                            });

                            // Renamed File

                            row.col(|ui| {
                                ui.label(choice_name.map_or("".to_owned(), |reference| {
                                    self.rename(&item_name, &reference)
                                }));
                            });
                        },
                    );
                });
            //});
        });
    }
}
