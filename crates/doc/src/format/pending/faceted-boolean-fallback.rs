//! The model cache a design keeps records how each boolean was made, and a boolean of bodies
//! that meet along faces that curve is now made a way no release has saved: the faceted
//! fallback (#1865), recorded as the strategy `faceted_fallback` with the tolerance the bodies
//! were faceted to and whether the result carries the operands' face history
//! (`references_kept`).
//!
//! An older design has neither: no build before this one could take that route, so none of its
//! booleans was made this way. A design that lacks `references_kept` on a retry it does have is
//! read as keeping the references, which every other retry does.
//!
//! The last release does not meet a design with this in it: it asks for an update instead,
//! because this change raises the format number.
