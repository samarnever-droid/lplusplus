/* Phase 5B/5C differential link shim: implements the runtime symbols the
 * 5B/5C slice can import, matching the native runtime's compact ARC layout.
 *
 * The common header is 16 bytes: magic@0, refcount@4, generation@8, flags@12.
 * Objects with a destructor store its 8-byte pointer immediately before that
 * common header, so their total prefix remains 24 bytes; objects without a
 * destructor (including strings) need only 16 bytes. The HAS_DTOR flag keeps
 * deallocation and cycle-collector teardown safe after header invalidation.
 * Immortal literal blobs carry the magic and immortal sentinels in the first
 * two words and are read-only, so retain/release never write them.
 *
 * Lists are private-layout ARC objects (header + LppList payload);
 * elements are stored one-per-8-byte-slot and interpreted per the push
 * kind. Out-of-bounds element access fails the link test loudly.
 */
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define LPP_SHIM_IMMORTAL 0x41524331u

typedef void (*LppShimDestructor)(void *payload);
typedef struct {
    uint32_t magic;
    int32_t refcount;
    int32_t generation;
    uint32_t cycle_flags;
} LppShimHeader;
_Static_assert(sizeof(LppShimHeader) == 16, "compact ARC header layout changed");

#define LPP_SHIM_HAS_DTOR 4u

static LppShimDestructor lpp_shim_destructor(LppShimHeader *header) {
    if (!(header->cycle_flags & LPP_SHIM_HAS_DTOR)) return (LppShimDestructor)0;
    return *(LppShimDestructor *)((char *)header - sizeof(LppShimDestructor));
}

static void *lpp_shim_allocation_base(LppShimHeader *header) {
    if (header->cycle_flags & LPP_SHIM_HAS_DTOR) {
        return (char *)header - sizeof(LppShimDestructor);
    }
    return header;
}

static inline int lpp_shim_is_immortal(const LppShimHeader *header) {
    return header->magic == LPP_SHIM_IMMORTAL &&
           (uint32_t)header->refcount == LPP_SHIM_IMMORTAL;
}

/* Monotonic source of object generations, matching the v1 runtime
 * (lpp__generation_counter): every live object carries a unique non-zero
 * generation, and a reused address never reuses one, so a stale weak handle
 * can never match a new occupant. Generation 0 is reserved as the
 * "untracked" sentinel used by the slice family. */
static int lpp_shim_generation_counter = 1;

void *lpp_arc_alloc_with_destructor(int64_t payload_size, LppShimDestructor destructor) {
    if (payload_size < 0) return (void *)0;
    size_t extra = destructor ? sizeof(LppShimDestructor) : 0;
    size_t prefix = sizeof(LppShimHeader) + extra;
    if ((uint64_t)payload_size > SIZE_MAX - prefix) return (void *)0;
    char *memory = (char *)calloc(1, prefix + (size_t)payload_size);
    if (!memory) return (void *)0;
    LppShimHeader *header = (LppShimHeader *)(memory + extra);
    header->magic = LPP_SHIM_IMMORTAL;
    header->refcount = 1;
    header->generation = lpp_shim_generation_counter++;
    header->cycle_flags = destructor ? LPP_SHIM_HAS_DTOR : 0;
    if (destructor) *(LppShimDestructor *)memory = destructor;
    return (char *)header + sizeof(LppShimHeader);
}

#include "c_shim/arc_cycle.inc"
#include "c_shim/lists.inc"
#include "c_shim/closures_tasks.inc"
#include "c_shim/builtins_strings.inc"
#include "c_shim/slices.inc"
#include "c_shim/simd.inc"
#include "c_shim/ffi.inc"
