//! A construction plane, axis or point can pick `sketch_crossing` as one of its references: where
//! two curves `a` and `b` of one `sketch` cross, with `near` (a sketch coordinate) naming which
//! crossing when there are several; it is found again on every rebuild. Older designs need no
//! transformation, they have none of these; the last release asks for an update instead of
//! meeting a reference it can't read.
