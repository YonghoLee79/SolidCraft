//! A canvas saves a sideways and a lengthways stretch (`scale_x`, `scale_y`, default 1; the shown
//! width is `width * scale_x`) and a top-to-bottom flip (`flip_v`). Older designs need no
//! transformation: a canvas without them is unstretched and unflipped, as before. The last
//! release refuses a design saved with this (the newer format number).
