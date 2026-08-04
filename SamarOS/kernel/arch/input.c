/* SamarOS — PS/2 keyboard + mouse, PIT millisecond clock and CMOS RTC.
 *
 * The kernel runs with interrupts masked and polls the 8042 controller from
 * the compositor loop; that keeps the whole system a single deterministic
 * run loop, which is exactly what the L++ UI layer wants.
 */
#include "samar.h"

/* ---- keyboard ------------------------------------------------------ */
#define KEYQ 64
static int  keyq[KEYQ];
static int  kq_head, kq_tail;
static int  shift_down, ctrl_down, caps_lock;

#define K_LEFT  0x101
#define K_RIGHT 0x102
#define K_UP    0x103
#define K_DOWN  0x104
#define K_DEL   0x105
#define K_HOME  0x106
#define K_END   0x107
#define K_SUPER 0x108
#define K_F(n)  (0x110 + (n))

static const char map_lower[128] = {
    0,  27, '1','2','3','4','5','6','7','8','9','0','-','=', 8, 9,
    'q','w','e','r','t','y','u','i','o','p','[',']', 13, 0, 'a','s',
    'd','f','g','h','j','k','l',';','\'','`', 0, '\\','z','x','c','v',
    'b','n','m',',','.','/', 0, '*', 0, ' ', 0, 0, 0, 0, 0, 0,
};

static const char map_upper[128] = {
    0,  27, '!','@','#','$','%','^','&','*','(',')','_','+', 8, 9,
    'Q','W','E','R','T','Y','U','I','O','P','{','}', 13, 0, 'A','S',
    'D','F','G','H','J','K','L',':','"','~', 0, '|','Z','X','C','V',
    'B','N','M','<','>','?', 0, '*', 0, ' ', 0, 0, 0, 0, 0, 0,
};

static void key_push(int k)
{
    int n = (kq_head + 1) % KEYQ;
    if (n == kq_tail) return;
    keyq[kq_head] = k;
    kq_head = n;
}

int key_pop(void)
{
    if (kq_head == kq_tail) return 0;
    int k = keyq[kq_tail];
    kq_tail = (kq_tail + 1) % KEYQ;
    return k;
}

int key_ctrl(void) { return ctrl_down; }
int key_shift(void) { return shift_down; }

/* ---- mouse --------------------------------------------------------- */
static int mx, my, mbtn, mwheel;
static u8  packet[4];
static int pkt_i, pkt_len = 3;

int mouse_x(void) { return mx; }
int mouse_y(void) { return my; }
int mouse_buttons(void) { return mbtn; }
int mouse_wheel(void) { int w = mwheel; mwheel = 0; return w; }

static void wait_write(void) { for (int i = 0; i < 100000; i++) if (!(k_inb(0x64) & 2)) return; }
static void wait_read(void)  { for (int i = 0; i < 100000; i++) if (k_inb(0x64) & 1) return; }

static void ctl_cmd(u32 c) { wait_write(); k_outb(0x64, c); }
static void ctl_data(u32 c) { wait_write(); k_outb(0x60, c); }
static u32  ctl_read(void) { wait_read(); return k_inb(0x60); }

static void mouse_cmd(u32 c)
{
    ctl_cmd(0xD4);
    ctl_data(c);
    ctl_read();               /* swallow ACK */
}

void input_init(void)
{
    /* drain anything the BIOS left behind */
    for (int i = 0; i < 32 && (k_inb(0x64) & 1); i++) k_inb(0x60);

    ctl_cmd(0xA8);            /* enable auxiliary (mouse) port           */

    ctl_cmd(0x20);            /* read controller config byte             */
    u32 cfg = ctl_read();
    cfg |=  0x40;             /* scancode translation -> set 1           */
    cfg &= ~0x30u;            /* clocks on for both ports                */
    cfg &= ~0x03u;            /* polled: no IRQs                         */
    ctl_cmd(0x60);
    ctl_data(cfg);

    mouse_cmd(0xF6);          /* defaults                                */

    /* IntelliMouse knock sequence — unlocks the scroll wheel */
    mouse_cmd(0xF3); mouse_cmd(200);
    mouse_cmd(0xF3); mouse_cmd(100);
    mouse_cmd(0xF3); mouse_cmd(80);
    ctl_cmd(0xD4); ctl_data(0xF2); ctl_read();
    u32 id = ctl_read();
    pkt_len = (id == 3) ? 4 : 3;

    mouse_cmd(0xF3); mouse_cmd(60);   /* 60 samples/sec                  */
    mouse_cmd(0xF4);                  /* enable reporting                */

    ctl_cmd(0xAE);            /* enable keyboard port                    */

    mx = fb_width() / 2;
    my = fb_height() / 2;
}

static void handle_scancode(u32 sc)
{
    static int extended;
    if (sc == 0xE0) { extended = 1; return; }

    int release = sc & 0x80;
    int code = sc & 0x7F;

    if (extended) {
        extended = 0;
        if (release) { if (code == 0x1D) ctrl_down = 0; return; }
        switch (code) {
            case 0x4B: key_push(K_LEFT);  return;
            case 0x4D: key_push(K_RIGHT); return;
            case 0x48: key_push(K_UP);    return;
            case 0x50: key_push(K_DOWN);  return;
            case 0x53: key_push(K_DEL);   return;
            case 0x47: key_push(K_HOME);  return;
            case 0x4F: key_push(K_END);   return;
            case 0x5B: key_push(K_SUPER); return;
            case 0x1D: ctrl_down = 1;     return;
            default: return;
        }
    }

    if (code == 0x2A || code == 0x36) { shift_down = !release; return; }
    if (code == 0x1D) { ctrl_down = !release; return; }
    if (code == 0x3A) { if (!release) caps_lock = !caps_lock; return; }
    if (release) return;

    if (code >= 0x3B && code <= 0x44) { key_push(K_F(code - 0x3B + 1)); return; }

    int up = shift_down;
    char ch = up ? map_upper[code] : map_lower[code];
    if (caps_lock && ch >= 'a' && ch <= 'z') ch = (char)(ch - 32);
    else if (caps_lock && shift_down && ch >= 'A' && ch <= 'Z') ch = (char)(ch + 32);
    if (ch) key_push((int)(unsigned char)ch);
}

static void handle_mouse(u8 b)
{
    if (pkt_i == 0 && !(b & 0x08)) return;      /* resynchronise          */
    packet[pkt_i++] = b;
    if (pkt_i < pkt_len) return;
    pkt_i = 0;

    u8 f = packet[0];
    if (f & 0xC0) return;                        /* overflow — drop       */

    int dx = (int)packet[1] - ((f & 0x10) ? 256 : 0);
    int dy = (int)packet[2] - ((f & 0x20) ? 256 : 0);
    mx += dx;
    my -= dy;
    if (mx < 0) mx = 0;
    if (my < 0) my = 0;
    if (mx > fb_width() - 1)  mx = fb_width() - 1;
    if (my > fb_height() - 1) my = fb_height() - 1;
    mbtn = f & 7;

    if (pkt_len == 4) {
        s8 z = (s8)(packet[3] & 0x0F);
        if (z & 0x08) z |= (s8)0xF0;
        if (z) mwheel += -z;
    }
}

/* ---- PIT millisecond clock ---------------------------------------- */
static u16 pit_last;
static u32 pit_frac, pit_ms;

static void time_update(void)
{
    k_outb(0x43, 0x00);                 /* latch channel 0               */
    u32 lo = k_inb(0x40);
    u32 hi = k_inb(0x40);
    u16 cur = (u16)(lo | (hi << 8));
    u16 delta = (u16)(pit_last - cur);  /* channel 0 counts down         */
    pit_last = cur;
    pit_frac += delta;
    while (pit_frac >= 1193) { pit_frac -= 1193; pit_ms++; }
}

int time_ms(void) { return (int)pit_ms; }

void input_poll(void)
{
    time_update();
    for (int guard = 0; guard < 64; guard++) {
        u32 st = k_inb(0x64);
        if (!(st & 1)) break;
        u32 data = k_inb(0x60);
        if (st & 0x20) handle_mouse((u8)data);
        else           handle_scancode(data);
    }
}

/* ---- CMOS real time clock ------------------------------------------ */
static u32 cmos(u32 reg)
{
    k_outb(0x70, reg);
    return k_inb(0x71);
}

static u32 bcd(u32 v) { return (v & 0x0F) + ((v >> 4) * 10); }

static void rtc_read(int *h, int *m, int *s, int *day, int *mon, int *yr, int *wd)
{
    while (cmos(0x0A) & 0x80) { }
    u32 sec = cmos(0x00), min = cmos(0x02), hour = cmos(0x04);
    u32 wday = cmos(0x06), mday = cmos(0x07), month = cmos(0x08), year = cmos(0x09);
    u32 regb = cmos(0x0B);
    if (!(regb & 0x04)) {
        sec = bcd(sec); min = bcd(min); mday = bcd(mday);
        month = bcd(month); year = bcd(year); wday = bcd(wday);
        u32 pm = hour & 0x80;
        hour = bcd(hour & 0x7F);
        if (!(regb & 0x02) && pm) hour = (hour % 12) + 12;
    }
    *h = (int)hour; *m = (int)min; *s = (int)sec;
    *day = (int)mday; *mon = (int)month; *yr = (int)(2000 + year); *wd = (int)wday;
}

int rtc_hour(void) { int a,b,c,d,e,f,g; rtc_read(&a,&b,&c,&d,&e,&f,&g); return a; }
int rtc_min(void)  { int a,b,c,d,e,f,g; rtc_read(&a,&b,&c,&d,&e,&f,&g); return b; }
int rtc_sec(void)  { int a,b,c,d,e,f,g; rtc_read(&a,&b,&c,&d,&e,&f,&g); return c; }
int rtc_day(void)  { int a,b,c,d,e,f,g; rtc_read(&a,&b,&c,&d,&e,&f,&g); return d; }
int rtc_month(void){ int a,b,c,d,e,f,g; rtc_read(&a,&b,&c,&d,&e,&f,&g); return e; }
int rtc_year(void) { int a,b,c,d,e,f,g; rtc_read(&a,&b,&c,&d,&e,&f,&g); return f; }
int rtc_weekday(void) { int a,b,c,d,e,f,g; rtc_read(&a,&b,&c,&d,&e,&f,&g); return g; }
