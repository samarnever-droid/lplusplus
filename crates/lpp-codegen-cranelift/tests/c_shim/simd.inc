/* ── 5C2: the scalar SIMD checksum (the 17 vector operators are native) ──── */

int64_t lpp_vec_i64_checksum(int64_t n) {
    if (n < 0) return 0;
    int64_t total = 0;
    for (int64_t i = 0; i < n; i++) total += (i * 3) ^ (i >> 1);
    return total;
}
