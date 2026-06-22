use std::path::PathBuf;

pub fn pick_folder() -> Option<PathBuf> {
    rfd::FileDialog::new().pick_folder()
}

pub fn pick_csv_for_import() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("CSV", &["csv"])
        .pick_file()
}

pub fn pick_csv_for_save() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("CSV", &["csv"])
        .set_file_name("export.csv")
        .save_file()
}
