use super::component::*;
use super::event::{Event, Events};
use crate::ui::rectangles::Rectangles;

use crate::api::{Client, Downloads, UpdateChecker};
use crate::cache::Cache;
use crate::config::Config;
use crate::ui::*;
use crate::Messages;

use std::error::Error;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use signal_hook::consts::signal::*;
use signal_hook_tokio::Signals;
use termion::event::Key;
use tokio::sync::RwLock;
use tokio::task;

pub struct MainUI<'a> {
    cache: Cache,
    downloads: Downloads,
    events: Events,
    rectangles: Rectangles,
    focused: FocusedWidget<'a>,
    top_bar: Arc<RwLock<TopBar<'a>>>,
    files_view: Arc<RwLock<FileTable<'a>>>,
    download_view: Arc<RwLock<DownloadTable<'a>>>,
    msg_view: Arc<RwLock<MessageList<'a>>>,
    bottom_bar: Arc<RwLock<BottomBar<'a>>>,
    redraw_terminal: Arc<AtomicBool>,
    updater: UpdateChecker,
    msgs: Messages,
}

impl<'a> MainUI<'static> {
    pub fn new(cache: Cache, client: Client, config: Config, downloads: Downloads, msgs: Messages) -> Self {
        // TODO use Tokio events?
        let events = Events::new();
        let updater = UpdateChecker::new(cache.clone(), client.clone(), config, msgs.clone());

        let top_bar = RwLock::new(TopBar::new()).into();

        let redraw_terminal = Arc::new(AtomicBool::new(true));

        let files_view = Arc::new(RwLock::new(FileTable::new(redraw_terminal.clone(), cache.file_index.clone())));
        let download_view = RwLock::new(DownloadTable::new(redraw_terminal.clone(), downloads.clone())).into();
        let msg_view = RwLock::new(MessageList::new(redraw_terminal.clone(), msgs.clone())).into();
        let bottom_bar = RwLock::new(BottomBar::new(redraw_terminal.clone(), client.request_counter)).into();

        let focused = FocusedWidget::FileTable(files_view.clone());

        Self {
            cache,
            downloads,
            events,
            rectangles: Rectangles::new(),
            focused,
            top_bar,
            files_view,
            download_view,
            msg_view,
            bottom_bar,
            redraw_terminal,
            updater,
            msgs,
        }
    }

    /* This is the main UI loop.
     * Redrawing the terminal is quite CPU intensive, so we use a bunch of locks and atomics to make sure it only
     * happens when necessary. */
    pub async fn run(&mut self) -> Result<(), Box<dyn Error>> {
        self.files_view.write().await.focus().await;
        /* X11 (and maybe Wayland?) sends SIGWINCH when the window is resized, so we can listen to that. Otherwise we
         * redraw when something has changed.
         * We set this to true so that all widgets are rendered in the first loop. */
        let got_sigwinch = Arc::new(AtomicBool::new(true));
        let signals = Signals::new([SIGWINCH])?;
        let handle = signals.handle();
        let _sigwinch_task = task::spawn(handle_sigwinch(signals, got_sigwinch.clone()));
        let mut terminal = term_setup().unwrap();

        loop {
            {
                let mut files_view = self.files_view.write().await;
                let mut downloads_view = self.download_view.write().await;
                let mut msgs_view = self.msg_view.write().await;
                let mut topbar = self.top_bar.write().await;
                let mut botbar = self.bottom_bar.write().await;
                files_view.refresh().await;
                downloads_view.refresh().await;
                msgs_view.refresh().await;
                topbar.refresh().await;
                botbar.refresh().await;

                let recalculate_rects = got_sigwinch.swap(false, Ordering::Relaxed);

                if self.redraw_terminal.swap(false, Ordering::Relaxed) || recalculate_rects {
                    // TODO should this be done in a blocking thread?
                    terminal.draw(|f| {
                        if recalculate_rects {
                            self.rectangles.recalculate(f.size());
                        }
                        f.render_stateful_widget(
                            files_view.widget.clone(),
                            self.rectangles.rect_main[0],
                            &mut files_view.state,
                        );
                        f.render_stateful_widget(
                            downloads_view.widget.clone(),
                            self.rectangles.rect_main[1],
                            &mut downloads_view.state,
                        );
                        f.render_stateful_widget(
                            msgs_view.widget.clone(),
                            self.rectangles.rect_root[1],
                            &mut msgs_view.state,
                        );
                        f.render_widget(topbar.widget.clone(), self.rectangles.rect_topbar[0]);
                        f.render_widget(botbar.widget.clone(), self.rectangles.rect_botbar[1]);
                    })?;
                }
            }

            if let Ok(Event::Input(key)) = self.events.next() {
                if let Key::Char('q') | Key::Ctrl('c') = key {
                    handle.close();
                    return Ok(());
                } else {
                    self.handle_keypress(key).await;
                }
            }
        }
    }

    async fn handle_keypress(&mut self, key: Key) {
        match key {
            Key::Char('q') | Key::Ctrl('c') => {
                //handle.close();
                //return Ok(());
            }
            Key::Down | Key::Char('j') => {
                self.focused.next().await;
            }
            Key::Up | Key::Char('k') => {
                self.focused.previous().await;
            }
            Key::Left | Key::Char('h') => match self.focused {
                FocusedWidget::MessageList(_) | FocusedWidget::DownloadTable(_) => {
                    self.focused.change_to(FocusedWidget::FileTable(self.files_view.clone())).await;
                }
                FocusedWidget::FileTable(_) => {
                    self.focused.change_to(FocusedWidget::MessageList(self.msg_view.clone())).await;
                }
            },
            Key::Right | Key::Char('l') => match self.focused {
                FocusedWidget::MessageList(_) | FocusedWidget::FileTable(_) => {
                    self.focused.change_to(FocusedWidget::DownloadTable(self.download_view.clone())).await;
                }
                FocusedWidget::DownloadTable(_) => {
                    self.focused.change_to(FocusedWidget::MessageList(self.msg_view.clone())).await;
                }
            },
            Key::Char('i') => {
                if let FocusedWidget::FileTable(fv) = &self.focused {
                    let ftable_lock = fv.read().await;
                    if let Some(i) = ftable_lock.state.selected() {
                        self.updater.ignore_file(i).await;
                    }
                }
            }
            Key::Char('p') => {
                if let FocusedWidget::DownloadTable(_) = &self.focused {
                    let dls_table = self.download_view.read().await;
                    if let Some(i) = dls_table.state.selected() {
                        self.downloads.toggle_pause_for(i).await;
                    }
                }
            }
            Key::Char('U') => {
                if let FocusedWidget::FileTable(fv) = &self.focused {
                    let game: String;
                    let mod_id: u32;
                    {
                        let ftable_lock = fv.read().await;
                        if let Some(i) = ftable_lock.state.selected() {
                            let files_lock = ftable_lock.file_index.files_sorted.read().await;
                            let fdata = files_lock.get(i).unwrap();
                            let lf_lock = fdata.local_file.read().await;
                            game = lf_lock.game.clone();
                            mod_id = lf_lock.mod_id;
                        } else {
                            return;
                        }
                    }
                    self.updater.update_mod(game, mod_id).await;
                }
            }
            Key::Char('u') => {
                if let FocusedWidget::FileTable(_fv) = &self.focused {
                    self.updater.update_all().await;
                }
            }
            Key::Char('v') => {
                if let FocusedWidget::FileTable(fv) = &self.focused {
                    let ftable_lock = fv.read().await;
                    if let Some(i) = ftable_lock.state.selected() {
                        let files_lock = ftable_lock.file_index.files_sorted.read().await;
                        let fdata = files_lock.get(i).unwrap();
                        let lf_lock = fdata.local_file.read().await;
                        let url = format!("https://www.nexusmods.com/{}/mods/{}", &lf_lock.game, &lf_lock.mod_id);
                        if let Err(_) = Command::new("xdg-open").arg(url).status() {
                            self.msgs.push(format!("xdg-open is needed to open URLs in browser.")).await;
                        }
                    }
                }
            }
            Key::Delete => match &self.focused.clone() {
                FocusedWidget::FileTable(ft) => {
                    let mut ft_lock = ft.write().await;
                    if let Some(i) = ft_lock.state.selected() {
                        if let Err(e) = self.cache.delete_by_index(i).await {
                            self.msgs.push(format!("Unable to delete file: {}", e)).await;
                        } else {
                            if i == 0 {
                                ft_lock.state.select(None);
                            }
                            drop(ft_lock);
                            self.focused.previous().await;
                        }
                    }
                }
                FocusedWidget::DownloadTable(dt) => {
                    let mut dt_lock = dt.write().await;
                    if let Some(i) = dt_lock.state.selected() {
                        dt_lock.downloads.delete(i).await;
                        if i == 0 {
                            dt_lock.state.select(None);
                        }
                        drop(dt_lock);
                        self.focused.previous().await;
                    }
                }
                FocusedWidget::MessageList(ml) => {
                    let mut ml_lock = ml.write().await;
                    if let Some(i) = ml_lock.state.selected() {
                        ml_lock.msgs.remove(i).await;
                        if i == 0 {
                            ml_lock.state.select(None);
                        }
                        drop(ml_lock);
                        self.focused.previous().await;
                    }
                }
            },
            _ => {
                // Uncomment to log keypresses
                // self.msgs.push(format!("{:?}", key)).await;
            }
        }
    }
}
