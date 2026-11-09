use super::*;

pub struct SearchBox {
    pub widget: NewWidget<Flex>,
    pub field: Handle<TextEditor>,
    pub placeholder: Handle<Placeholder>,
    pub clear: Handle<IconButton>,
}

pub struct FindBox {
    pub widget: NewWidget<Flex>,
    pub field: Handle<TextEditor>,
    pub placeholder: Handle<Placeholder>,
}

