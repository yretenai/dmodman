use crate::cache::FileIndex;

use tokio_stream::StreamExt;
use tui::layout::Constraint;
use tui::style::{Color, Style};
use tui::widgets::{Block, Borders, Cell, Row, Table, TableState};

pub struct FileTable<'a> {
    headers: Row<'a>,
    pub block: Block<'a>,
    pub highlight_style: Style,
    pub files: FileIndex,
    pub state: TableState,
    pub widget: Table<'a>,
}

impl<'a> FileTable<'a> {
    pub fn new(files: &FileIndex) -> Self {
        let block = Block::default().borders(Borders::ALL).title("Files");
        let headers =
            Row::new(vec!["Name", "Version"].iter().map(|h| Cell::from(*h).style(Style::default().fg(Color::Red))));

        Self {
            block,
            files: files.clone(),
            headers,
            highlight_style: Style::default(),
            state: TableState::default(),
            widget: Table::new(vec![]),
        }
    }

    pub async fn refresh<'b>(&mut self)
    where
        'b: 'a, {
        let files = self.files.items().await;
        let mut stream = tokio_stream::iter(files);
        let mut rows: Vec<Row> = vec![];
        while let Some(file_details) = stream.next().await {
            rows.push(Row::new(vec![
                file_details.name.clone(),
                file_details.version.as_ref().unwrap_or(&"".to_string()).to_string(),
            ]))
        }

        self.widget = Table::new(rows)
            .header(self.headers.to_owned())
            .block(self.block.to_owned())
            .widths(&[Constraint::Percentage(85), Constraint::Percentage(15)])
            .highlight_style(self.highlight_style.to_owned());
    }
}
