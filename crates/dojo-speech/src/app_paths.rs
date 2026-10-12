use std::path::PathBuf;

pub fn support_dir() -> crate::Result<PathBuf> {
    if let Some(path) = std::env::var_os("DOJO_SPEECH_DATA_DIR") {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err(crate::error(
                "DOJO_SPEECH_DATA_DIR must be an absolute path",
            ));
        }
        return Ok(path);
    }
    dirs::data_dir()
        .map(|path| path.join("dojo").join("speech"))
        .ok_or_else(|| crate::error("could not determine the user data directory"))
}
