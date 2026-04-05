use std::io::{self, Stdout};
use crossterm::event::{self, KeyEventKind};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Widget;


fn main() -> io::Result<()> {
    let  mut terminal: Terminal<CrosstermBackend<Stdout>> = ratatui::init();

    let mut app = App{
        exit: false,
    };

    let app_result = app.run(&mut terminal);

    ratatui::restore();
    app_result
}


pub struct App{
    exit: bool,
}

impl App {
    fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
        while !self.exit {

            //Lit les événements de clavier et les traite
            match  crossterm::event::read()? {
                //Ajoute la fonction handle_key_event pour gérer les événements de clavier
                crossterm::event::Event::Key(key_event) => self.handle_key_event(key_event)?,
                _ => {}
            }


            terminal.draw(|frame: &mut ratatui::Frame<'_>| self.draw(frame))?;
        }

        Ok(())
    }

    fn draw(&self, frame: &mut ratatui::Frame<'_>) {
        frame.render_widget("Robots Game", frame.area());
    }


    fn handle_key_event(&mut self, key_event: crossterm::event::KeyEvent) -> io::Result<()> {
       
        // Quitte l'application si une touche est pressée
        if key_event.kind == KeyEventKind::Press {

            self.exit = true;
            
        }
        Ok(())
    }
}

impl Widget for &App {
    fn render(self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer)
        where
            Self: Sized {
        //Execute le rendu du widget
        Line::from("Ratatui Robots").bold().render(area, buf);
    }
}