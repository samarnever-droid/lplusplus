/* Compiler-private PGO helpers. Keep this hosted C reference ABI-compatible
 * with the Rust runtime's bounded dense counter table. */
#define LPP_PGO_COUNTER_CAP 1048576
static uint64_t lpp_pgo_counters[LPP_PGO_COUNTER_CAP];

void lpp_pgo_hit(int64_t function_id) {
    if (function_id < 0 || function_id >= LPP_PGO_COUNTER_CAP) return;
#if defined(__GNUC__) || defined(__clang__)
    __atomic_fetch_add(&lpp_pgo_counters[function_id], 1, __ATOMIC_RELAXED);
#else
    lpp_pgo_counters[function_id]++;
#endif
}

void lpp_pgo_flush(int64_t fingerprint) {
    const char *path = getenv("LPP_PGO_PROFILE");
    if (!path || !*path) path = "lpp.pgo";
    FILE *profile = fopen(path, "wb");
    if (!profile) return;
    fprintf(profile, "LPP-PGO-1 %016llx\n", (unsigned long long)(uint64_t)fingerprint);
    for (uint64_t id = 0; id < LPP_PGO_COUNTER_CAP; ++id) {
        uint64_t count = lpp_pgo_counters[id];
        if (count) fprintf(profile, "%llu %llu\n", (unsigned long long)id,
                           (unsigned long long)count);
    }
    fclose(profile);
}
