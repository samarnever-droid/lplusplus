/* Non-atomic ARC, emitted when the compiler proves the program never spawns a
 * thread. Normal objects bypass the arena registry entirely; only headers
 * explicitly tagged by lpp_arena_alloc pay for the lookup. */
void lpp_arc_retain_local(void *p) { if(!p)return; LppArcHeader *h=(LppArcHeader*)p-1; if(lpp__is_immortal(h))return; _InterlockedIncrement(&h->refcount); }
void lpp_arc_release_local(void *p) {
    if (!p) return;
    LppArcHeader *h = (LppArcHeader *)p - 1;
    if (lpp__is_immortal(h)) return;
    long left = _InterlockedDecrement(&h->refcount);
    if (left == 0) {
        int arena = lpp_win_is_arena_header(h);
        WinArenaRegion *region = arena ? lpp_arena_for_header(h) : 0;
        lpp__runtime_drop_count++;
        if (lpp__cycle_is_collectible(h)) lpp__cycle_deregister(p);
        void *base;
        LppArcDestructor destructor;
        lpp__teardown_info(h, &base, &destructor);
        h->generation++;
        if (destructor) destructor(p);
        if (region) lpp_arena_node_zero(region);
        else if (!arena) lpp_arc_free_base(h, base);
    } else if (left > 0) {
        lpp__note_possible_root(p);
    }
}
void *lpp_alloc(int64_t sz){return lpp_arc_alloc(sz);}
void lpp_free(void *p,int64_t sz){(void)sz;lpp_arc_release(p);}
void lpp_closure_destroy(void *c){if(c)lpp_arc_release(((void**)c)[1]);}
void lpp_closure_trace(void *c,void *ctx){if(c)lpp_trace_visit(ctx,((void**)c)[1]);}
/* This freestanding runtime has no reusable small-block allocator, so an
 * address is never recycled after VirtualFree during a process lifetime. The
 * compiler also rejects source reassignment while a view is live. */
int64_t lpp_weak_generation(void*p){return p?1:0;}
int64_t lpp_weak_get(int64_t raw,int64_t generation){return generation==1?raw:0;}

typedef struct{uint64_t managed_mask,packed_offsets;}LppTuplePrefix;
static void lpp_tuple_destroy(void*p){LppTuplePrefix*t=(LppTuplePrefix*)p;unsigned i;for(i=0;t&&i<4;i++){if(!(t->managed_mask&((uint64_t)1<<i)))continue;uint64_t o=(t->packed_offsets>>(i*16))&0xffff;lpp_arc_release(*(void**)((char*)p+o));}}
void lpp_tuple_trace(void*p,void*ctx){LppTuplePrefix*t=(LppTuplePrefix*)p;unsigned i;for(i=0;t&&i<4;i++){if(!(t->managed_mask&((uint64_t)1<<i)))continue;uint64_t o=(t->packed_offsets>>(i*16))&0xffff;lpp_trace_visit(ctx,*(void**)((char*)p+o));}}
void*lpp_tuple_alloc(int64_t size,int64_t mask,int64_t offsets){LppTuplePrefix*t;if(size<16)ExitProcess(101);t=(LppTuplePrefix*)lpp_arc_alloc_with_destructor(size,lpp_tuple_destroy);if(!t)ExitProcess(101);t->managed_mask=(uint64_t)mask;t->packed_offsets=(uint64_t)offsets;return t;}
typedef int64_t(*LppTaskCode)(void*);
typedef struct{LppTaskCode code;void*environment;int64_t result;volatile long state;long result_managed;}LppTask;
static void lpp_task_payload_destroy(void*p){LppTask*t=(LppTask*)p;if(!t)return;if(t->environment){lpp_arc_release(t->environment);t->environment=0;}if(_InterlockedExchangeAdd(&t->state,0)==2&&t->result_managed&&t->result){lpp_arc_release((void*)(intptr_t)t->result);t->result=0;}}
void lpp_task_trace(void*p,void*ctx){LppTask*t=(LppTask*)p;if(!t)return;lpp_trace_visit(ctx,t->environment);if(_InterlockedExchangeAdd(&t->state,0)==2&&t->result_managed&&t->result)lpp_trace_visit(ctx,(void*)(intptr_t)t->result);}
void*lpp_task_new(void*code,void*environment,int64_t managed){LppTask*t;if(!code||!environment)ExitProcess(101);t=(LppTask*)lpp_arc_alloc_with_destructor(sizeof(LppTask),lpp_task_payload_destroy);if(!t)ExitProcess(101);t->code=(LppTaskCode)code;t->environment=environment;t->result_managed=managed!=0;t->state=0;return t;}
int64_t lpp_task_poll(void*raw){LppTask*t=(LppTask*)raw;long observed;if(!t)ExitProcess(101);observed=_InterlockedExchangeAdd(&t->state,0);if(observed==2)return 1;if(_InterlockedCompareExchange(&t->state,1,0)!=0){if(_InterlockedExchangeAdd(&t->state,0)==2)return 1;ExitProcess(101);}t->result=t->code(t->environment);_InterlockedExchange(&t->state,2);return 1;}
int64_t lpp_executor_run(void*raw){(void)lpp_task_poll(raw);return((LppTask*)raw)->result;}
int64_t lpp_task_await(void*raw){LppTask*t=(LppTask*)raw;int64_t r=lpp_executor_run(raw);if(t->result_managed&&r)lpp_arc_retain((void*)(intptr_t)r);return r;}
void lpp_task_destroy(void*raw){lpp_arc_release(raw);}

static void lpp_list_destroy(void *p) { LppList *l=(LppList*)p; if(!l)return; if(l->arc_elements){int64_t i;for(i=0;i<l->len;i++)lpp_arc_release((void*)(intptr_t)l->data[i]);} if(l->data)VirtualFree(l->data,0,MEM_RELEASE); }
void lpp_list_trace(void *p,void *ctx) { LppList *l=(LppList*)p; int64_t i; if(!l||!l->arc_elements)return; for(i=0;i<l->len;i++)lpp_trace_visit(ctx,(void*)(intptr_t)l->data[i]); }
static void *lpp_list_new_with_mode(int ae) { LppList *l=(LppList*)lpp_arc_alloc_with_destructor((int64_t)sizeof(LppList),lpp_list_destroy); if(!l)return 0; l->arc_elements=ae; return l; }
void *lpp_list_new(void){return lpp_list_new_with_mode(0);}
void *lpp_list_new_arc(void){return lpp_list_new_with_mode(1);}
void lpp_list_push(void *r,int64_t v){LppList*l=(LppList*)r;if(!l)return;if(l->len==l->cap){int64_t nc=l->cap==0?8:l->cap*2;if(nc<l->cap||nc>(int64_t)(0x7fffffffffffffffLL/8))return;uint64_t nb=lpp_page_round((uint64_t)nc*sizeof(int64_t));int64_t*nd=(int64_t*)VirtualAlloc(0,nb,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);if(!nd)return;int64_t i;for(i=0;i<l->len;i++)nd[i]=l->data[i];if(l->data)VirtualFree(l->data,0,MEM_RELEASE);l->data=nd;l->cap=nc;l->data_bytes=nb;} if(l->arc_elements)lpp_arc_retain((void*)(intptr_t)v);l->data[l->len++]=v;}
void lpp_list_push_arc(void*l,void*v){if(l)((LppList*)l)->arc_elements=1;lpp_list_push(l,(int64_t)(intptr_t)v);}
void lpp_list_push_float(void*l,double v){int64_t i;lpp_memcpy((char*)&i,(const char*)&v,8);lpp_list_push(l,i);}
void lpp_list_push_bool(void*l,int8_t v){lpp_list_push(l,v?1:0);}
int64_t lpp_list_get(void*r,int64_t i){LppList*l=(LppList*)r;if(!l||i<0||i>=l->len){ExitProcess(101);}return l->data[i];}
void lpp_list_set(void*r,int64_t i,int64_t v){LppList*l=(LppList*)r;if(!l||i<0||i>=l->len)ExitProcess(101);if(l->arc_elements){lpp_arc_retain((void*)(intptr_t)v);lpp_arc_release((void*)(intptr_t)l->data[i]);}l->data[i]=v;}
void lpp_list_set_bool(void*l,int64_t i,int8_t v){lpp_list_set(l,i,v?1:0);}
void lpp_list_set_float(void*l,int64_t i,double v){int64_t b;lpp_memcpy((char*)&b,(const char*)&v,8);lpp_list_set(l,i,b);}
void lpp_list_set_arc(void*l,int64_t i,void*v){if(l)((LppList*)l)->arc_elements=1;lpp_list_set(l,i,(int64_t)(intptr_t)v);}
double lpp_list_get_float(void*l,int64_t idx){int64_t i=lpp_list_get(l,idx);double f;lpp_memcpy((char*)&f,(const char*)&i,8);return f;}
int8_t lpp_list_get_bool(void*l,int64_t i){return lpp_list_get(l,i)!=0;}
void *lpp_list_get_arc(void*l,int64_t i){return(void*)(intptr_t)lpp_list_get(l,i);}
int64_t lpp_list_len(void*r){return r?((LppList*)r)->len:0;}
void lpp_panic_div_zero(void){ExitProcess(101);}
int64_t lpp_list_pop(void*r){LppList*l=(LppList*)r;if(!l||l->len<=0)ExitProcess(101);l->len--;return l->data[l->len];}
void lpp_list_free(void*l){lpp_arc_release(l);}
void lpp_list_insert(void*r,int64_t idx,int64_t v){LppList*l=(LppList*)r;if(!l||idx<0||idx>l->len)ExitProcess(101);lpp_list_push(l,0);for(int64_t i=l->len-1;i>idx;i--)l->data[i]=l->data[i-1];l->data[idx]=v;if(l->arc_elements)lpp_arc_retain((void*)(intptr_t)v);}
int64_t lpp_list_remove(void*r,int64_t idx){LppList*l=(LppList*)r;if(!l||idx<0||idx>=l->len)ExitProcess(101);int64_t val=l->data[idx];for(int64_t i=idx;i<l->len-1;i++)l->data[i]=l->data[i+1];l->len--;return val;}
void lpp_list_reserve(void*r,int64_t cap){(void)r;(void)cap;}
int64_t lpp_list_capacity(void*r){return r?((LppList*)r)->cap:0;}
void lpp_list_clear(void*r){LppList*l=(LppList*)r;if(!l)return;if(l->arc_elements){for(int64_t i=0;i<l->len;i++)lpp_arc_release((void*)(intptr_t)l->data[i]);}l->len=0;}
void lpp_list_truncate(void*r,int64_t n){LppList*l=(LppList*)r;if(!l||n>=l->len||n<0)return;if(l->arc_elements){for(int64_t i=n;i<l->len;i++)lpp_arc_release((void*)(intptr_t)l->data[i]);}l->len=n;}
void lpp_list_swap(void*r,int64_t i,int64_t j){LppList*l=(LppList*)r;if(!l||i<0||i>=l->len||j<0||j>=l->len)ExitProcess(101);int64_t t=l->data[i];l->data[i]=l->data[j];l->data[j]=t;}
void lpp_list_reverse(void*r){LppList*l=(LppList*)r;if(!l||l->len<=1)return;int64_t i=0,j=l->len-1;while(i<j){int64_t t=l->data[i];l->data[i]=l->data[j];l->data[j]=t;i++;j--;}}
void lpp_list_sort(void*r){LppList*l=(LppList*)r;if(!l||l->len<2)return;for(int64_t i=1;i<l->len;i++){int64_t k=l->data[i];int64_t j=i-1;while(j>=0&&l->data[j]>k){l->data[j+1]=l->data[j];j--;}l->data[j+1]=k;}}
void lpp_list_sort_desc(void*r){LppList*l=(LppList*)r;if(!l||l->len<2)return;for(int64_t i=1;i<l->len;i++){int64_t k=l->data[i];int64_t j=i-1;while(j>=0&&l->data[j]<k){l->data[j+1]=l->data[j];j--;}l->data[j+1]=k;}}
void lpp_list_sort_u(void*r){LppList*l=(LppList*)r;if(!l||l->len<2)return;for(int64_t i=1;i<l->len;i++){int64_t k=l->data[i];int64_t j=i-1;while(j>=0&&(uint64_t)l->data[j]>(uint64_t)k){l->data[j+1]=l->data[j];j--;}l->data[j+1]=k;}}
int64_t lpp_list_index_of(void*r,int64_t v){LppList*l=(LppList*)r;if(!l)return -1;for(int64_t i=0;i<l->len;i++){if(l->data[i]==v)return i;}return -1;}
int64_t lpp_list_binary_search(void*r,int64_t v){LppList*l=(LppList*)r;if(!l)return -1;int64_t lo=0,hi=l->len-1;while(lo<=hi){int64_t mid=lo+(hi-lo)/2;if(l->data[mid]==v)return mid;if(l->data[mid]<v)lo=mid+1;else hi=mid-1;}return -(lo+1);}
void lpp_list_extend(void*d,void*s){LppList*dst=(LppList*)d;LppList*src=(LppList*)s;if(!dst||!src)return;for(int64_t i=0;i<src->len;i++)lpp_list_push(dst,src->data[i]);}
void lpp_prefetch(int64_t addr){(void)addr;}
void lpp_prefetch_write(int64_t addr){(void)addr;}
typedef struct{int64_t v[2];}LppVec128;
LppVec128 lpp_vec_i64x2_and(LppVec128 a,LppVec128 b){LppVec128 r;r.v[0]=a.v[0]&b.v[0];r.v[1]=a.v[1]&b.v[1];return r;}
LppVec128 lpp_vec_i64x2_or(LppVec128 a,LppVec128 b){LppVec128 r;r.v[0]=a.v[0]|b.v[0];r.v[1]=a.v[1]|b.v[1];return r;}
LppVec128 lpp_vec_i64x2_not(LppVec128 a){LppVec128 r;r.v[0]=~a.v[0];r.v[1]=~a.v[1];return r;}
LppVec128 lpp_vec_u8x16_splat(int64_t byte_val){uint8_t b=(uint8_t)byte_val;uint64_t half=0;for(int i=0;i<8;i++)half|=((uint64_t)b<<(i*8));LppVec128 r;r.v[0]=(int64_t)half;r.v[1]=(int64_t)half;return r;}
LppVec128 lpp_vec_u8x16_eq(LppVec128 a,LppVec128 b){uint8_t*pa=(uint8_t*)&a;uint8_t*pb=(uint8_t*)&b;LppVec128 r;uint8_t*pr=(uint8_t*)&r;for(int i=0;i<16;i++)pr[i]=(pa[i]==pb[i])?0xFF:0x00;return r;}
int64_t lpp_vec_u8x16_movemask(LppVec128 a){uint8_t*pa=(uint8_t*)&a;int64_t m=0;for(int i=0;i<16;i++){if(pa[i]&0x80)m|=(1<<i);}return m;}
typedef struct{void*base;int64_t start,length,generation,kind;}LppSlice;
static void*lpp_slice_checked_base(LppSlice*v){int64_t r;if(!v||!v->base||!v->generation)ExitProcess(101);r=lpp_weak_get((int64_t)(intptr_t)v->base,v->generation);if(!r)ExitProcess(101);return(void*)(intptr_t)r;}
void*lpp_slice_init(void*storage,void*base,int64_t start,int64_t length,int64_t kind){int64_t n;LppSlice*v;if(!storage||!base||start<0||length<0||start>0x7fffffffffffffffLL-length)ExitProcess(101);n=kind==0?(int64_t)lpp_strlen((const char*)base):lpp_list_len(base);if(start>n||length>n-start)ExitProcess(101);v=(LppSlice*)storage;v->base=base;v->start=start;v->length=length;v->generation=lpp_weak_generation(base);v->kind=kind;return v;}
int64_t lpp_slice_len(void*raw){LppSlice*v=(LppSlice*)raw;(void)lpp_slice_checked_base(v);return v->length;}
int64_t lpp_slice_get(void*raw,int64_t index){LppSlice*v=(LppSlice*)raw;void*b=lpp_slice_checked_base(v);if(v->kind!=1||index<0||index>=v->length)ExitProcess(101);return lpp_list_get(b,v->start+index);}
double lpp_slice_get_float(void*raw,int64_t index){int64_t i=lpp_slice_get(raw,index);double f;lpp_memcpy((char*)&f,(const char*)&i,8);return f;}
int8_t lpp_slice_get_bool(void*raw,int64_t index){return lpp_slice_get(raw,index)!=0;}
char*lpp_str_slice_get(void*raw,int64_t index){LppSlice*v=(LppSlice*)raw;const char*b=(const char*)lpp_slice_checked_base(v);char*r;if(v->kind!=0||index<0||index>=v->length)ExitProcess(101);r=(char*)lpp_arc_alloc(2);if(!r)ExitProcess(101);r[0]=b[v->start+index];r[1]=0;return r;}
char*lpp_str_slice_to_str(void*raw){LppSlice*v=(LppSlice*)raw;const char*b=(const char*)lpp_slice_checked_base(v);char*r;if(v->kind!=0)ExitProcess(101);r=(char*)lpp_arc_alloc(v->length+1);if(!r)ExitProcess(101);lpp_memcpy(r,b+v->start,(int)v->length);r[v->length]=0;return r;}

