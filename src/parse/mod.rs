//! Decoder output parsers: multimon-ng (POCSAG/APRS), dsd-neo (digital
//! voice), SBS/BaseStation (ADS-B). All tolerant — decoder builds vary.

pub mod ais;
pub mod dsd;
pub mod ft8;
pub mod multimon;
pub mod sbs;
