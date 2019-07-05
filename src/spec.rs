#[derive(Eq, PartialEq, Debug)]
pub enum Type {
    Mark,
    Ignore,
}

#[derive(Eq, PartialEq, Debug)]
pub enum Spec {
    Line(usize),
    Range(usize, usize),
    Regex(String),
}

#[derive(Eq, PartialEq, Debug)]
pub enum Target {
    Dir(String),
    File(String),
    FileSpec(String, Spec),
}

#[derive(Eq, PartialEq, Debug)]
pub struct Marking {
    pub target: Target,
    pub mark_type: Type,
}

impl Marking {
    pub fn new(target: Target, mark_type: Type) -> Self {
        Marking { target, mark_type }
    }
}
