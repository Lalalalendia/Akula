use aquilum_core::settings::models::AppConfig;
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Index(usize),
    Number(f64),
    Text(String),
}

#[derive(Clone, Copy)]
pub enum Setter {
    Config(fn(&mut AppConfig, &Value)),
    Scale,
    HomePage,
    Autostart,
}

pub(crate) fn same_setter(a: Setter, b: Setter) -> bool {
    match (a, b) {
        (Setter::Config(x), Setter::Config(y)) => std::ptr::fn_addr_eq(x, y),
        (Setter::Scale, Setter::Scale) | (Setter::HomePage, Setter::HomePage) | (Setter::Autostart, Setter::Autostart) => true,
        _ => false,
    }
}
