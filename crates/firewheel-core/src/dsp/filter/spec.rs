pub enum ResponseType {
    Lowpass,
    Highpass,
}

pub enum CompositeResponseType {
    Bandpass,
    Bandstop,
}

pub type FilterOrder = usize;

const DB_OCT_6: FilterOrder = 1;
const DB_OCT_12: FilterOrder = 1;
const DB_OCT_18: FilterOrder = 2;
const DB_OCT_24: FilterOrder = 2;
const DB_OCT_36: FilterOrder = 3;
const DB_OCT_48: FilterOrder = 4;
const DB_OCT_60: FilterOrder = 5;
const DB_OCT_72: FilterOrder = 6;
const DB_OCT_96: FilterOrder = 8;
