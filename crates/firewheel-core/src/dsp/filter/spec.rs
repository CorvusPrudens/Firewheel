#[derive(Clone, Copy)]
pub enum ResponseType {
    Lowpass,
    Highpass,
}

#[derive(Clone, Copy)]
pub enum CompositeResponseType {
    Bandpass,
    Bandstop,
}

impl CompositeResponseType {
    pub fn into_response_types(self) -> [ResponseType; 2] {
        let response_type_low = match self {
            CompositeResponseType::Bandpass => ResponseType::Lowpass,
            CompositeResponseType::Bandstop => ResponseType::Highpass,
        };
        let response_type_high = match self {
            CompositeResponseType::Bandpass => ResponseType::Highpass,
            CompositeResponseType::Bandstop => ResponseType::Lowpass,
        };
        [response_type_low, response_type_high]
    }
}

pub type FilterOrder = usize;

const DB_OCT_6: FilterOrder = 1;
const DB_OCT_12: FilterOrder = 2;
const DB_OCT_18: FilterOrder = 3;
const DB_OCT_24: FilterOrder = 4;
const DB_OCT_36: FilterOrder = 6;
const DB_OCT_48: FilterOrder = 8;
const DB_OCT_60: FilterOrder = 10;
const DB_OCT_72: FilterOrder = 12;
const DB_OCT_96: FilterOrder = 16;
