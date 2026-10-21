//! A canvas saves `see_through` (drawn over the model) and `selectable` (default true: clicking
//! it in the view selects it), each written only when it differs from the default. Older designs
//! need no transformation: a canvas without them is hidden by the model and selectable, as
//! before. The last release refuses a design saved with this (the newer format number).
