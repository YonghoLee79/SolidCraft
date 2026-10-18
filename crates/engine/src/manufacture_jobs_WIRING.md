# Companion CAM UI wiring

This branch stages `manufacture_jobs` + `cmd/manufacture/jobs.rs` from the Manufacture megapr.

Full `CompanionCamBackend` UI pump wires after:
1. #2620 workspace shell
2. #2645 solidcraft-cam app
3. Expanding `manufacture.rs` commands beyond settings get/set

Until then these modules are present for review but not registered in `cmd/mod.rs`.
