#[derive(Clone, Copy)]
pub enum ResponseType {
    Simple(SimpleResponseType),
    Composite(CompositeResponseType),
}

#[derive(Clone, Copy)]
pub enum SimpleResponseType {
    Lowpass,
    Highpass,
}

#[derive(Clone, Copy)]
pub enum CompositeResponseType {
    Bandpass,
    Bandstop,
}

impl CompositeResponseType {
    pub fn into_response_types(self) -> [SimpleResponseType; 2] {
        let response_type_low = match self {
            CompositeResponseType::Bandpass => SimpleResponseType::Lowpass,
            CompositeResponseType::Bandstop => SimpleResponseType::Highpass,
        };
        let response_type_high = match self {
            CompositeResponseType::Bandpass => SimpleResponseType::Highpass,
            CompositeResponseType::Bandstop => SimpleResponseType::Lowpass,
        };
        [response_type_low, response_type_high]
    }
}

pub type FilterOrder = usize;

pub const DB_OCT_6: FilterOrder = 1;
pub const DB_OCT_12: FilterOrder = 2;
pub const DB_OCT_18: FilterOrder = 3;
pub const DB_OCT_24: FilterOrder = 4;
pub const DB_OCT_36: FilterOrder = 6;
pub const DB_OCT_48: FilterOrder = 8;
pub const DB_OCT_72: FilterOrder = 12;
pub const DB_OCT_96: FilterOrder = 16;
