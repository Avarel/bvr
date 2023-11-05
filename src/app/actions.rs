pub enum Action {
    Command(CommandAction),
    Viewer(ViewerAction),
}

pub enum ViewerAction {
    ScrollUp,
    ScrollDown,
}

pub enum CommandAction {
    CursorMove {
        direction: CursorDirection,
        select: bool,
        jump: CursorJump,
    },
    Type(char),
    Paste(String),
}

pub enum CursorJump {
    Word,
    Boundary,
    None,
}

pub enum CursorDirection {
    Left,
    Right,
}
