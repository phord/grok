use crate::config::Config;
use crate::display::Display;
use crate::keyboard::{Input, UserCommand};
use indexed_file::indexer::LogFile;

pub struct Viewer {
    config: Config,
    display: Display,
    input: Input,
    file: LogFile,
}

impl Viewer {
    pub fn new(config: Config) -> Self {
        let filename = config.filename.get(0).expect("No filename specified").clone();
        let file = LogFile::new(Some(filename)).expect("Failed to open file");
        println!("{:?}", file);

        Self {
            config: config.clone(),
            display: Display::new(config),
            input: Input::new(),
            file,
        }
    }

    pub fn run(&mut self) -> crossterm::Result<bool> {

        self.display.set_length(self.file.lines());

        let lines = self.display.lines_needed();
        for row in lines {
            let line = self.file.readline(row as usize);
            if let Some(line) = line {
                self.display.push(row, &line);
            }
        }

        // self.display.clear();
        self.display.refresh_screen()?;

        let cmd = self.input.get_command()?;

        match cmd {
            UserCommand::Quit => Ok(false),
            UserCommand::SearchPrompt => {
                self.display.search_prompt();
                Ok(true)
            }
            _ => {
                self.display.handle_command(cmd);
                Ok(true)
            }
        }
    }

}

impl Drop for Viewer {
    fn drop(&mut self) {
        // Output::clear_screen().expect("Error");
    }
}
