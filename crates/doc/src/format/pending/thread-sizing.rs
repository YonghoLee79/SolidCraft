//! Thread features save `sizing: "iso261"` for new automatic sizing. It snaps external
//! diameters to ISO 261 presets and falls back to the measured diameter when none fits;
//! internal threads keep tap-drill sizing. An absent field means the historical rule,
//! so existing automatic features preserve their sizes and still recompute as before.
//! Explicit designations retain their meaning. Older releases refuse the newer format
//! instead of silently evaluating new automatic features with the historical table.
