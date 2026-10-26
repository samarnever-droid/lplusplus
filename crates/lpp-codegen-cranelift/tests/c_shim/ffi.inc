/* Tiny native target for the cross-module C ABI regression. */
int64_t lpp_test_imported_ffi_add(int64_t left, int64_t right) {
    return left + right;
}

/* PGO imports are exercised through the Rust runtime in the driver gate; keep
 * the unit-test C shim linkable for object-level instrumentation checks. */
void lpp_pgo_hit(int64_t function_id) { (void)function_id; }
void lpp_pgo_flush(int64_t fingerprint) { (void)fingerprint; }
