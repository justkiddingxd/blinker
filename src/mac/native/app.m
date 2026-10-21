// Process, windows, monitors, input, tray icon, global hotkeys, signals.

#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>
#import <Carbon/Carbon.h>
#import <mach/mach.h>
#import <malloc/malloc.h>
#import <notify.h>
#import <objc/runtime.h>
#import <objc/message.h>
#include "bk.h"

void bk_set_err(char *buf, int len, NSString *msg) {
    if (!buf || len <= 0) return;
    const char *s = msg.UTF8String ?: "error";
    strlcpy(buf, s, (size_t)len);
}

NSString *bk_str(const char *s) { return s ? [NSString stringWithUTF8String:s] : @""; }

void bk_on_main(dispatch_block_t b) {
    if ([NSThread isMainThread]) b();
    else dispatch_async(dispatch_get_main_queue(), b);
}

int bk_is_main_thread(void) { return [NSThread isMainThread] ? 1 : 0; }

/// Appends a line to ~/Library/Logs/Blinker/blinker.log (an app from Finder
/// has no terminal for its errors).
void bk_log(const char *line) {
    static dispatch_queue_t q;
    static NSString *path;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        q = dispatch_queue_create("blinker.log", DISPATCH_QUEUE_SERIAL);
        NSString *dir = [NSHomeDirectory() stringByAppendingPathComponent:@"Library/Logs/Blinker"];
        [NSFileManager.defaultManager createDirectoryAtPath:dir withIntermediateDirectories:YES attributes:nil error:nil];
        path = [dir stringByAppendingPathComponent:@"blinker.log"];
    });
    NSString *text = [NSString stringWithFormat:@"%@ [%d] %s\n", [NSDate date], getpid(), line ?: ""];
    dispatch_async(q, ^{
        NSFileHandle *h = [NSFileHandle fileHandleForWritingAtPath:path];
        if (!h) {
            [NSFileManager.defaultManager createFileAtPath:path contents:nil attributes:nil];
            h = [NSFileHandle fileHandleForWritingAtPath:path];
        }
        [h seekToEndOfFile];
        [h writeData:[text dataUsingEncoding:NSUTF8StringEncoding]];
        [h closeFile];
    });
}

static void logf_(NSString *fmt, ...) NS_FORMAT_FUNCTION(1, 2);
static void logf_(NSString *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    NSString *s = [[NSString alloc] initWithFormat:fmt arguments:ap];
    va_end(ap);
    bk_log(s.UTF8String);
}

void bk_free(void *p) { free(p); }

// ---------------------------------------------------------------------------
// Coordinates: Cocoa (bottom-left origin) <-> top-left global points.

static double primary_height(void) {
    NSArray<NSScreen *> *screens = NSScreen.screens;
    if (screens.count == 0) return CGDisplayBounds(CGMainDisplayID()).size.height;
    return screens[0].frame.size.height;
}

static NSRect to_cocoa(double x, double y, double w, double h) {
    return NSMakeRect(x, primary_height() - (y + h), w, h);
}

static void from_cocoa(NSRect r, double *x, double *y, double *w, double *h) {
    *x = r.origin.x;
    *y = primary_height() - (r.origin.y + r.size.height);
    *w = r.size.width;
    *h = r.size.height;
}

// ---------------------------------------------------------------------------
// Process

void bk_set_activation_policy(int regular) {
    bk_on_main(^{
        [NSApp setActivationPolicy:regular ? NSApplicationActivationPolicyRegular : NSApplicationActivationPolicyAccessory];
    });
}

enum { BK_WAKE = 0x424b };

static void post_wake(void) {
    NSEvent *e = [NSEvent otherEventWithType:NSEventTypeApplicationDefined
                                    location:NSZeroPoint
                                   modifierFlags:0
                                   timestamp:0
                                windowNumber:0
                                     context:nil
                                     subtype:BK_WAKE
                                       data1:0
                                       data2:0];
    [NSApp postEvent:e atStart:YES];
}

/// The controller is idling in `bk_run_idle` (main thread only).
static BOOL g_idle;

/// Runs the application normally until `bk_wake_main` (main thread). Between
/// our windows the app must really be running: the menu bar icon (hosted by
/// Control Center on recent macOS) only gets its clicks through `-run`.
void bk_run_idle(void) {
    @autoreleasepool {
        if (!NSApp) {
            [NSThread sleepForTimeInterval:0.5];
            return;
        }
        g_idle = YES;
        [NSApp run];
        g_idle = NO;
    }
}

/// Ends a `bk_run_idle` (any thread). While a window of ours runs the event
/// loop instead (winit), nothing is stopped: the command waits for it.
void bk_wake_main(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        if (!g_idle) return;
        g_idle = NO;
        [NSApp stop:nil];
        post_wake();
    });
}

void bk_trim_memory(void) { malloc_zone_pressure_relief(NULL, 0); }

// ---------------------------------------------------------------------------
// Monitors and the pointer

int bk_displays(bk_display *out, int max) {
    __block int n = 0;
    void (^fill)(void) = ^{
        double h0 = primary_height();
        for (NSScreen *s in NSScreen.screens) {
            if (n >= max) break;
            bk_display *d = &out[n++];
            NSRect f = s.frame, v = s.visibleFrame;
            d->x = f.origin.x;
            d->y = h0 - (f.origin.y + f.size.height);
            d->w = f.size.width;
            d->h = f.size.height;
            d->vx = v.origin.x;
            d->vy = h0 - (v.origin.y + v.size.height);
            d->vw = v.size.width;
            d->vh = v.size.height;
            d->scale = s.backingScaleFactor;
            d->refresh = (double)s.maximumFramesPerSecond;
            d->safe_top = s.safeAreaInsets.top;
            d->id = [s.deviceDescription[@"NSScreenNumber"] unsignedIntValue];
        }
    };
    if ([NSThread isMainThread]) fill();
    else dispatch_sync(dispatch_get_main_queue(), fill);
    return n;
}

void bk_cursor_pos(double *x, double *y) {
    CGEventRef e = CGEventCreate(NULL);
    CGPoint p = e ? CGEventGetLocation(e) : CGPointZero;
    if (e) CFRelease(e);
    *x = p.x;
    *y = p.y;
}

int bk_key_down(uint16_t keycode) {
    return CGEventSourceKeyState(kCGEventSourceStateCombinedSessionState, keycode) ? 1 : 0;
}

// Key presses of our own windows (for the hotkey recorder in settings).
static uint16_t g_key;
static uint64_t g_key_flags;
static double g_key_at = -1;

void bk_install_key_monitor(void) {
    bk_on_main(^{
        static id monitor;
        if (monitor) return;
        monitor = [NSEvent addLocalMonitorForEventsMatchingMask:NSEventMaskKeyDown
                                                        handler:^NSEvent *(NSEvent *e) {
                                                            if (!e.isARepeat) {
                                                                g_key = e.keyCode;
                                                                g_key_flags = e.modifierFlags;
                                                                g_key_at = e.timestamp;
                                                            }
                                                            return e;
                                                        }];
    });
}

int bk_take_key_press(uint16_t *keycode, uint64_t *flags) {
    if (g_key_at < 0) return 0;
    double age = NSProcessInfo.processInfo.systemUptime - g_key_at;
    g_key_at = -1;
    if (age > 0.4) return 0;
    *keycode = g_key;
    *flags = g_key_flags;
    return 1;
}

// ---------------------------------------------------------------------------
// Our windows (NSWindow * from winit)

void *bk_window_of_view(void *view) {
    NSView *v = (__bridge NSView *)view;
    return (__bridge void *)v.window;
}

/// Every window of ours: colors are sRGB (our screenshots are converted to
/// it), so a frozen screen looks exactly like the live one.
void bk_window_init(void *w) {
    NSWindow *win = (__bridge NSWindow *)w;
    bk_on_main(^{
        win.colorSpace = NSColorSpace.sRGBColorSpace;
    });
}

/// One of our windows by its title (egui's extra viewports have no handle).
void *bk_window_find(const char *title) {
    NSString *t = bk_str(title);
    for (NSWindow *w in NSApp.windows)
        if ([w.title isEqualToString:t]) return (__bridge void *)w;
    return NULL;
}

void bk_window_frame(void *w, double *x, double *y, double *ww, double *hh) {
    NSWindow *win = (__bridge NSWindow *)w;
    from_cocoa(win.frame, x, y, ww, hh);
}

void bk_window_set_frame(void *w, double x, double y, double ww, double hh, int topmost) {
    NSWindow *win = (__bridge NSWindow *)w;
    bk_on_main(^{
        NSRect f = to_cocoa(x, y, ww, hh);
        if (!NSEqualRects(win.frame, f)) [win setFrame:f display:YES animate:NO];
        if (topmost && win.level < NSFloatingWindowLevel) win.level = NSFloatingWindowLevel;
    });
}

double bk_window_scale(void *w) {
    NSWindow *win = (__bridge NSWindow *)w;
    return win.backingScaleFactor > 0 ? win.backingScaleFactor : 1.0;
}

void bk_window_set_alpha(void *w, double a) {
    NSWindow *win = (__bridge NSWindow *)w;
    bk_on_main(^{ win.alphaValue = a; });
}

void bk_window_activate(void *w) {
    NSWindow *win = (__bridge NSWindow *)w;
    bk_on_main(^{
        // macOS 14 activation is cooperative; a hotkey press counts as the
        // user asking for us
        if (@available(macOS 14.0, *)) [NSApp activate];
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
        [NSApp activateIgnoringOtherApps:YES];
#pragma clang diagnostic pop
        [win makeKeyAndOrderFront:nil];
        [win orderFrontRegardless];
    });
}

int bk_window_is_key(void *w) {
    NSWindow *win = (__bridge NSWindow *)w;
    return (win.isKeyWindow && NSApp.isActive) ? 1 : 0;
}

void bk_window_show(void *w, int show) {
    NSWindow *win = (__bridge NSWindow *)w;
    bk_on_main(^{
        if (show) [win orderFront:nil];
        else [win orderOut:nil];
    });
}

/// Undecorated window with the system look: rounded corners, shadow, dark.
// ---------------------------------------------------------------------------
// System window buttons over our own title bar

/// The system title bar lets every click but the ones on its buttons through
/// to our bar under it (it would take them for dragging). Done the way
/// `keep_open` is: a subclass of its own class that still reports the original.
static void pass_clicks_through(NSView *v) {
    if (!v) return;
    Class cls = object_getClass(v);
    NSString *name = NSStringFromClass(cls);
    if ([name hasPrefix:@"BKPass_"]) return;
    NSString *subName = [@"BKPass_" stringByAppendingString:name];
    Class sub = NSClassFromString(subName);
    if (!sub) {
        sub = objc_allocateClassPair(cls, subName.UTF8String, 0);
        if (!sub) return;
        class_addMethod(sub, @selector(hitTest:), imp_implementationWithBlock(^NSView *(NSView *self_, NSPoint p) {
                            struct objc_super sup = {self_, cls};
                            NSView *(*send)(struct objc_super *, SEL, NSPoint) = (void *)objc_msgSendSuper;
                            NSView *hit = send(&sup, @selector(hitTest:), p);
                            return [hit isKindOfClass:NSButton.class] ? hit : nil;
                        }),
                        "@@:{CGPoint=dd}");
        Class shown = [v class];
        class_addMethod(sub, @selector(class), imp_implementationWithBlock(^Class(id self_) {
                            (void)self_;
                            return shown;
                        }),
                        "#@:");
        objc_registerClassPair(sub);
    }
    object_setClass(v, sub);
}

/// Keeps the close / minimize / zoom buttons in the middle of our title bar:
/// AppKit lays them out for its own 28 pt bar again on several occasions.
@interface BKWindowButtons : NSObject
@property(nonatomic, weak) NSWindow *window;
@property(nonatomic) CGFloat x;
@property(nonatomic) CGFloat cy;
@end

@implementation BKWindowButtons
- (void)apply {
    NSWindow *w = self.window;
    NSButton *close = [w standardWindowButton:NSWindowCloseButton];
    NSView *container = close.superview.superview;
    if (!w || !close || !container) return;
    CGFloat h = self.cy * 2.0;
    CGFloat x = self.x;
    for (NSNumber *kind in @[@(NSWindowCloseButton), @(NSWindowMiniaturizeButton), @(NSWindowZoomButton)]) {
        NSButton *b = [w standardWindowButton:(NSWindowButton)kind.integerValue];
        if (!b) continue;
        [b setFrameOrigin:NSMakePoint(x, round((h - NSHeight(b.frame)) / 2.0))];
        x += NSWidth(b.frame) + 6.0;
    }
    container.frame = NSMakeRect(0, NSHeight(w.frame) - h, NSWidth(w.frame), h);
    w.titlebarSeparatorStyle = NSTitlebarSeparatorStyleNone;
    pass_clicks_through(container);
}
- (void)changed:(NSNotification *)n {
    (void)n;
    [self apply];
    // some relayouts happen after the notification
    dispatch_async(dispatch_get_main_queue(), ^{ [self apply]; });
}
@end

static char kBKWindowButtons;

void bk_window_buttons_at(void *w, double x, double cy) {
    NSWindow *win = (__bridge NSWindow *)w;
    bk_on_main(^{
        BKWindowButtons *b = objc_getAssociatedObject(win, &kBKWindowButtons);
        if (!b) {
            b = [BKWindowButtons new];
            b.window = win;
            objc_setAssociatedObject(win, &kBKWindowButtons, b, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
            NSNotificationCenter *nc = NSNotificationCenter.defaultCenter;
            for (NSNotificationName name in @[NSWindowDidResizeNotification, NSWindowDidBecomeKeyNotification,
                                              NSWindowDidResignKeyNotification, NSWindowDidChangeBackingPropertiesNotification,
                                              NSWindowDidDeminiaturizeNotification, NSWindowDidExposeNotification])
                [nc addObserver:b selector:@selector(changed:) name:name object:win];
        }
        b.x = x;
        b.cy = cy;
        [b changed:nil];
    });
}

// ---------------------------------------------------------------------------
// Liquid Glass (macOS 26): a glass pane behind a part of our (transparent) GL
// view, the way Finder's sidebar floats in its window

int bk_glass_available(void) {
    if (@available(macOS 26.0, *)) return 1;
    return 0;
}

static char kBKGlass;

/// A glass pane at (x, y, w, h) points from the window's top left, under
/// the egui view, which has to leave that part transparent. `bg`: the
/// window gets this background around it (0 = none, for floating cards).
int bk_window_glass(void *w, double x, double y, double width, double height, double radius, uint32_t bg) {
    if (@available(macOS 26.0, *)) {
        NSWindow *win = (__bridge NSWindow *)w;
        bk_on_main(^{
            NSView *content = win.contentView;
            NSView *frame = content.superview;
            if (!frame) return;
            NSGlassEffectView *glass = objc_getAssociatedObject(win, &kBKGlass);
            if (!glass) {
                glass = [[NSGlassEffectView alloc] initWithFrame:NSZeroRect];
                glass.style = NSGlassEffectViewStyleRegular;
                [frame addSubview:glass positioned:NSWindowBelow relativeTo:content];
                objc_setAssociatedObject(win, &kBKGlass, glass, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
            }
            glass.cornerRadius = radius;
            CGFloat top = NSHeight(frame.bounds);
            glass.frame = NSMakeRect(x, top - y - height, width, height);
            win.opaque = NO;
            win.backgroundColor = bg ? [NSColor colorWithSRGBRed:((bg >> 16) & 0xFF) / 255.0
                                                          green:((bg >> 8) & 0xFF) / 255.0
                                                           blue:(bg & 0xFF) / 255.0
                                                          alpha:1.0]
                                     : NSColor.clearColor;
            content.wantsLayer = YES;
            content.layer.opaque = NO;
        });
        return 1;
    }
    return 0;
}

void bk_window_style_borderless(void *w) {
    NSWindow *win = (__bridge NSWindow *)w;
    bk_on_main(^{
        win.appearance = [NSAppearance appearanceNamed:NSAppearanceNameDarkAqua];
        // a titled window (system buttons over our bar) has the system's own
        // rounded corners, shadow and hairline
        if (win.styleMask & NSWindowStyleMaskTitled) {
            win.collectionBehavior |= NSWindowCollectionBehaviorFullScreenNone;
            return;
        }
        win.hasShadow = YES;
        win.opaque = NO;
        win.backgroundColor = NSColor.clearColor;
        NSView *v = win.contentView;
        v.wantsLayer = YES;
        v.layer.cornerRadius = 10.0;
        v.layer.cornerCurve = kCACornerCurveContinuous;
        v.layer.masksToBounds = YES;
        v.layer.borderWidth = 1.0 / MAX(win.backingScaleFactor, 1.0);
        v.layer.borderColor = [NSColor colorWithWhite:1.0 alpha:0.12].CGColor;
        [win invalidateShadow];
    });
}

/// Popups that animate themselves: no zoom-in, no system shadow.
void bk_window_plain(void *w, int shadow) {
    NSWindow *win = (__bridge NSWindow *)w;
    bk_on_main(^{
        win.animationBehavior = NSWindowAnimationBehaviorNone;
        win.hasShadow = shadow ? YES : NO;
        win.collectionBehavior |= NSWindowCollectionBehaviorCanJoinAllSpaces | NSWindowCollectionBehaviorFullScreenAuxiliary |
                                  NSWindowCollectionBehaviorIgnoresCycle;
    });
}

/// The capture overlay: above the menu bar, the Dock and full-screen apps.
void bk_window_make_overlay(void *w) {
    NSWindow *win = (__bridge NSWindow *)w;
    bk_on_main(^{
        win.level = NSScreenSaverWindowLevel;
        win.animationBehavior = NSWindowAnimationBehaviorNone;
        win.hasShadow = NO;
        win.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces | NSWindowCollectionBehaviorFullScreenAuxiliary |
                                 NSWindowCollectionBehaviorStationary | NSWindowCollectionBehaviorIgnoresCycle;
    });
}

// The app that was in front before one of our windows took focus.
void *bk_focus_take(void) {
    NSRunningApplication *f = NSWorkspace.sharedWorkspace.frontmostApplication;
    if (!f || f.processIdentifier == getpid()) return NULL;
    return (__bridge_retained void *)f;
}

void bk_focus_restore(void *p) {
    if (!p) return;
    NSRunningApplication *app = (__bridge_transfer NSRunningApplication *)p;
    if (!NSApp.isActive) return;
    NSWindow *key = NSApp.keyWindow;
    if (key && key.isVisible) return;
    [app activateWithOptions:0];
}

// ---------------------------------------------------------------------------
// Shell helpers

void bk_message_box(const char *title, const char *text) {
    NSString *t = bk_str(title), *m = bk_str(text);
    void (^show)(void) = ^{
        NSAlert *a = [NSAlert new];
        a.alertStyle = NSAlertStyleWarning;
        a.messageText = t;
        a.informativeText = m;
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
        [NSApp activateIgnoringOtherApps:YES];
#pragma clang diagnostic pop
        [a runModal];
    };
    if ([NSThread isMainThread]) show();
    else dispatch_sync(dispatch_get_main_queue(), show);
}

void bk_reveal(const char *path) {
    NSURL *u = [NSURL fileURLWithPath:bk_str(path)];
    [NSWorkspace.sharedWorkspace activateFileViewerSelectingURLs:@[ u ]];
}

void bk_open_url(const char *url) {
    NSURL *u = [NSURL URLWithString:bk_str(url)];
    if (u) [NSWorkspace.sharedWorkspace openURL:u];
}

void bk_open(const char *path) { [NSWorkspace.sharedWorkspace openURL:[NSURL fileURLWithPath:bk_str(path)]]; }

int bk_system_is_russian(void) {
    NSSet *ru = [NSSet setWithArray:@[ @"ru", @"uk", @"be", @"kk" ]];
    for (NSString *l in NSLocale.preferredLanguages) {
        NSString *code = [l componentsSeparatedByString:@"-"].firstObject.lowercaseString;
        if ([ru containsObject:code]) return 1;
    }
    NSString *region = NSLocale.currentLocale.countryCode;
    if (region && [[NSSet setWithArray:@[ @"RU", @"UA", @"BY", @"KZ" ]] containsObject:region]) return 1;
    CFArrayRef sources = TISCreateInputSourceList(NULL, false);
    int found = 0;
    if (sources) {
        for (CFIndex i = 0; i < CFArrayGetCount(sources) && !found; i++) {
            TISInputSourceRef s = (TISInputSourceRef)CFArrayGetValueAtIndex(sources, i);
            CFArrayRef langs = TISGetInputSourceProperty(s, kTISPropertyInputSourceLanguages);
            if (!langs) continue;
            for (NSString *l in (__bridge NSArray *)langs) {
                if ([ru containsObject:l.lowercaseString]) {
                    found = 1;
                    break;
                }
            }
        }
        CFRelease(sources);
    }
    return found;
}

// ---------------------------------------------------------------------------
// Signals between blinker processes (Darwin notifications)

typedef void (*bk_notify_cb)(int which);

int bk_notify_listen(const char *name, int which, bk_notify_cb cb) {
    int token = 0;
    return notify_register_dispatch(name, &token, dispatch_get_main_queue(), ^(int t) {
               (void)t;
               cb(which);
           }) == NOTIFY_STATUS_OK
               ? 1
               : 0;
}

void bk_notify_post(const char *name) { notify_post(name); }

int bk_notify_token(const char *name) {
    int token = -1;
    if (notify_register_check(name, &token) != NOTIFY_STATUS_OK) return -1;
    int check = 0;
    notify_check(token, &check);  // the first check always reports a change
    return token;
}

int bk_notify_check(int token) {
    int check = 0;
    if (notify_check(token, &check) != NOTIFY_STATUS_OK) return 0;
    return check;
}

// ---------------------------------------------------------------------------
// Tray icon (status item) and its menu

/// `clicks`: 1, or 2 for a double click (only told apart when asked to).
typedef void (*bk_tray_cb)(int right, int clicks);
typedef void (*bk_menu_cb)(int id);

/// Left clicks are told apart from double clicks (set from the settings).
static int g_tray_double;
/// Counts left clicks: a pending single click is dropped when another comes.
static unsigned g_tray_clicks;

void bk_tray_set_double(int on) {
    g_tray_double = on;
}

typedef struct {
    int id;          // 0 = separator
    int depth;       // 0 = top level, 1 = inside the submenu above, -1 = a tile of the top row
    int submenu;     // opens the following depth+1 items
    const char *label;
    const char *symbol;  // SF Symbol name or NULL
    const char *key;     // key equivalent ("2") or NULL
    uint64_t mods;       // NSEventModifierFlags for the key equivalent
} bk_menu_item;

@interface BKTarget : NSObject
@property(nonatomic) bk_tray_cb tray;
@property(nonatomic) bk_menu_cb menu;
@property(nonatomic) bk_notify_cb reopen;
@end

@implementation BKTarget
- (void)clicked:(id)sender {
    (void)sender;
    NSEvent *e = NSApp.currentEvent;
    int right = e.type == NSEventTypeRightMouseUp || e.type == NSEventTypeRightMouseDown ||
                (e.modifierFlags & NSEventModifierFlagControl) != 0;
    if (!self.tray) return;
    if (right || !g_tray_double) {
        self.tray(right, 1);
        return;
    }
    // a double click wanted: the single one waits for the double-click time
    unsigned gen = ++g_tray_clicks;
    if (e.clickCount >= 2) {
        self.tray(0, 2);
        return;
    }
    bk_tray_cb cb = self.tray;
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(NSEvent.doubleClickInterval * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
        if (gen == g_tray_clicks) cb(0, 1);
    });
}
- (void)picked:(NSMenuItem *)item {
    if (self.menu) self.menu((int)item.tag);
}
- (void)handleReopen:(NSAppleEventDescriptor *)event withReply:(NSAppleEventDescriptor *)reply {
    (void)event;
    (void)reply;
    if (self.reopen) self.reopen(0);
}
@end

static NSStatusItem *g_item;
static BKTarget *g_target;

/// The app is opened again (Finder, Launchpad, Dock) while it runs: macOS
/// doesn't start a second process, it sends this one "reopen".
void bk_on_reopen(bk_notify_cb cb) {
    bk_on_main(^{
        if (!g_target) g_target = [BKTarget new];
        g_target.reopen = cb;
        [NSAppleEventManager.sharedAppleEventManager setEventHandler:g_target
                                                         andSelector:@selector(handleReopen:withReply:)
                                                       forEventClass:kCoreEventClass
                                                          andEventID:kAEReopenApplication];
    });
}

static NSImage *g_tray_image;
static NSString *g_tray_tip;

/// winit closes every window of the app when its event loop ends (after each
/// of our egui windows), the status item's own window included, and its
/// clicks then go nowhere. That one window ignores `close`: it gets a
/// subclass of its own class (the way KVO does it) that still reports the
/// original class.
static void keep_open(NSWindow *w) {
    if (!w) return;
    Class cls = object_getClass(w);
    NSString *name = NSStringFromClass(cls);
    if ([name hasPrefix:@"BKKeepOpen_"]) return;
    NSString *subName = [@"BKKeepOpen_" stringByAppendingString:name];
    Class sub = NSClassFromString(subName);
    if (!sub) {
        sub = objc_allocateClassPair(cls, subName.UTF8String, 0);
        if (!sub) return;
        class_addMethod(sub, @selector(close), imp_implementationWithBlock(^(id self_) { (void)self_; }), "v@:");
        Class shown = [w class];
        class_addMethod(sub, @selector(class), imp_implementationWithBlock(^Class(id self_) {
                            (void)self_;
                            return shown;
                        }),
                        "#@:");
        objc_registerClassPair(sub);
    }
    object_setClass(w, sub);
}

static void make_status_item(void) {
    g_item = [NSStatusBar.systemStatusBar statusItemWithLength:NSSquareStatusItemLength];
    // the same place in the menu bar when it is made again
    g_item.autosaveName = @"blinker";
    g_item.button.image = g_tray_image;
    g_item.button.toolTip = g_tray_tip ?: @"blinker";
    g_item.button.target = g_target;
    g_item.button.action = @selector(clicked:);
    [g_item.button sendActionOn:NSEventMaskLeftMouseUp | NSEventMaskRightMouseUp];
    keep_open(g_item.button.window);
}

static NSImage *tray_image(const uint8_t *rgba, int w, int h, int template_) {
    NSBitmapImageRep *rep = [[NSBitmapImageRep alloc] initWithBitmapDataPlanes:NULL
                                                                    pixelsWide:w
                                                                    pixelsHigh:h
                                                                 bitsPerSample:8
                                                               samplesPerPixel:4
                                                                      hasAlpha:YES
                                                                      isPlanar:NO
                                                                colorSpaceName:NSDeviceRGBColorSpace
                                                                  bitmapFormat:NSBitmapFormatAlphaNonpremultiplied
                                                                   bytesPerRow:w * 4
                                                                  bitsPerPixel:32];
    memcpy(rep.bitmapData, rgba, (size_t)w * h * 4);
    NSImage *img = [[NSImage alloc] initWithSize:NSMakeSize(18, 18)];
    [img addRepresentation:rep];
    img.template = template_ ? YES : NO;
    return img;
}

void bk_tray_create(const uint8_t *rgba, int w, int h, bk_tray_cb cb) {
    NSImage *img = tray_image(rgba, w, h, 1);
    bk_on_main(^{
        if (!g_target) g_target = [BKTarget new];
        g_target.tray = cb;
        g_tray_image = img;
        if (!g_item) make_status_item();
    });
}

/// Another icon (a red one while the instant replay records); `template_`
/// = drawn by the system in the menu bar's own color.
void bk_tray_set_image(const uint8_t *rgba, int w, int h, int template_, const char *tip) {
    NSImage *img = tray_image(rgba, w, h, template_);
    NSString *t = bk_str(tip);
    bk_on_main(^{
        g_tray_image = img;
        if (!g_item) return;
        g_tray_tip = t;
        g_item.button.image = img;
        g_item.button.toolTip = t;
    });
}

/// Before one of our egui windows: the icon's window (AppKit may have made a
/// new one) keeps out of winit's closing.
void bk_tray_protect(void) {
    bk_on_main(^{
        if (g_item) keep_open(g_item.button.window);
    });
}

/// After it, should the icon's window have been closed anyway: a fresh item
/// in its place.
void bk_tray_revive(void) {
    bk_on_main(^{
        if (!g_item || g_item.button.window.isVisible) return;
        [NSStatusBar.systemStatusBar removeStatusItem:g_item];
        make_status_item();
        logf_(@"tray: icon made again after a window closed");
    });
}

void bk_tray_remove(void) {
    bk_on_main(^{
        if (g_item) [NSStatusBar.systemStatusBar removeStatusItem:g_item];
        g_item = nil;
    });
}

// ---------------------------------------------------------------------------
// The top row of the tray menu: big tiles for the main actions

@interface BKTile : NSView
@property(nonatomic) int tag_;
@property(nonatomic) BOOL hot;
@property(nonatomic, weak) NSMenuItem *item;
@end

@implementation BKTile
- (instancetype)initWithSymbol:(NSString *)symbol title:(NSString *)title tag:(int)tag {
    self = [super initWithFrame:NSMakeRect(0, 0, 66, 60)];
    self.tag_ = tag;
    NSImageSymbolConfiguration *conf = [NSImageSymbolConfiguration configurationWithPointSize:19 weight:NSFontWeightRegular];
    NSImage *img = [[NSImage imageWithSystemSymbolName:symbol accessibilityDescription:title] imageWithSymbolConfiguration:conf];
    NSImageView *iv = [NSImageView imageViewWithImage:img ?: [NSImage new]];
    iv.contentTintColor = NSColor.labelColor;
    iv.frame = NSMakeRect(0, 26, 66, 26);
    iv.imageScaling = NSImageScaleNone;
    [self addSubview:iv];
    NSTextField *l = [NSTextField labelWithString:title];
    l.font = [NSFont systemFontOfSize:11];
    l.textColor = NSColor.secondaryLabelColor;
    l.alignment = NSTextAlignmentCenter;
    l.lineBreakMode = NSLineBreakByTruncatingTail;
    l.frame = NSMakeRect(2, 7, 62, 15);
    [self addSubview:l];
    [self addTrackingArea:[[NSTrackingArea alloc] initWithRect:NSZeroRect
                                                       options:NSTrackingMouseEnteredAndExited | NSTrackingActiveAlways | NSTrackingInVisibleRect
                                                         owner:self
                                                      userInfo:nil]];
    return self;
}
- (void)drawRect:(NSRect)r {
    (void)r;
    if (!self.hot) return;
    [[NSColor.labelColor colorWithAlphaComponent:0.1] setFill];
    [[NSBezierPath bezierPathWithRoundedRect:NSInsetRect(self.bounds, 2, 2) xRadius:8 yRadius:8] fill];
}
- (void)mouseEntered:(NSEvent *)e {
    (void)e;
    self.hot = YES;
    self.needsDisplay = YES;
}
- (void)mouseExited:(NSEvent *)e {
    (void)e;
    self.hot = NO;
    self.needsDisplay = YES;
}
- (void)mouseUp:(NSEvent *)e {
    (void)e;
    int tag = self.tag_;
    [self.item.menu cancelTracking];
    dispatch_async(dispatch_get_main_queue(), ^{
        if (g_target.menu) g_target.menu(tag);
    });
}
@end

static NSMenuItem *make_tiles(const bk_menu_item *items, int n) {
    NSMenuItem *row = [NSMenuItem new];
    CGFloat pad = 10, w = 66;
    NSView *v = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, pad * 2 + w * n, 60 + 8)];
    for (int i = 0; i < n; i++) {
        BKTile *t = [[BKTile alloc] initWithSymbol:bk_str(items[i].symbol) title:bk_str(items[i].label) tag:items[i].id];
        t.item = row;
        t.frame = NSMakeRect(pad + w * i, 4, w, 60);
        if (items[i].key) {
            NSString *k = bk_str(items[i].key).uppercaseString;
            NSEventModifierFlags m = (NSEventModifierFlags)items[i].mods;
            NSString *glyphs = [NSString stringWithFormat:@"%@%@%@%@%@", (m & NSEventModifierFlagControl) ? @"⌃" : @"",
                                (m & NSEventModifierFlagOption) ? @"⌥" : @"", (m & NSEventModifierFlagShift) ? @"⇧" : @"",
                                (m & NSEventModifierFlagCommand) ? @"⌘" : @"", k];
            t.toolTip = [NSString stringWithFormat:@"%@  %@", bk_str(items[i].label), glyphs];
        }
        [v addSubview:t];
    }
    row.view = v;
    return row;
}

/// The tray menu while it is open.
static NSMenu *g_open_menu;

static NSMenuItem *make_item(const bk_menu_item *it) {
    NSMenuItem *m = [[NSMenuItem alloc] initWithTitle:bk_str(it->label) action:nil keyEquivalent:@""];
    if (it->symbol) {
        NSImage *img = [NSImage imageWithSystemSymbolName:bk_str(it->symbol) accessibilityDescription:nil];
        if (img) m.image = img;
    }
    if (it->key) {
        m.keyEquivalent = bk_str(it->key);
        m.keyEquivalentModifierMask = (NSEventModifierFlags)it->mods;
    }
    if (!it->submenu) {
        m.tag = it->id;
        m.target = g_target;
        m.action = @selector(picked:);
    }
    return m;
}

/// Shows the menu under the tray icon. The chosen item is reported through `cb`.
void bk_tray_menu(const bk_menu_item *items, int n, bk_menu_cb cb) {
    if (!g_item) return;
    g_target.menu = cb;
    NSMenu *root = [NSMenu new];
    root.autoenablesItems = NO;
    NSMenu *sub = nil;
    // the tiles come first, as one row
    int tiles = 0;
    while (tiles < n && items[tiles].depth < 0) tiles++;
    if (tiles > 0) {
        [root addItem:make_tiles(items, tiles)];
        [root addItem:NSMenuItem.separatorItem];
    }
    for (int i = tiles; i < n; i++) {
        const bk_menu_item *it = &items[i];
        NSMenu *into = it->depth > 0 && sub ? sub : root;
        if (it->id == 0 && !it->submenu) {
            [into addItem:NSMenuItem.separatorItem];
            continue;
        }
        NSMenuItem *m = make_item(it);
        [into addItem:m];
        if (it->submenu) {
            sub = [NSMenu new];
            sub.autoenablesItems = NO;
            m.submenu = sub;
        }
    }
    // opened after the click that asked for it has been handled
    dispatch_async(dispatch_get_main_queue(), ^{
        g_item.menu = root;
        g_open_menu = root;
        [g_item.button performClick:nil];
        g_open_menu = nil;
        g_item.menu = nil;
    });
}

/// Closes the tray menu if it is open: a hotkey pressed while it shows
/// (the main thread is inside the menu until it closes).
void bk_tray_menu_close(void) {
    NSMenu *m = g_open_menu;
    if (m) [m cancelTrackingWithoutAnimation];
}

// ---------------------------------------------------------------------------
// Global hotkeys (Carbon: no accessibility permission needed)

typedef void (*bk_hotkey_cb)(uint32_t id);
static bk_hotkey_cb g_hotkey_cb;
static NSMutableDictionary<NSNumber *, NSValue *> *g_hotkeys;

static OSStatus hotkey_handler(EventHandlerCallRef next, EventRef ev, void *ud) {
    (void)next;
    (void)ud;
    EventHotKeyID hk;
    if (GetEventParameter(ev, kEventParamDirectObject, typeEventHotKeyID, NULL, sizeof(hk), NULL, &hk) == noErr && g_hotkey_cb)
        g_hotkey_cb(hk.id);
    return noErr;
}

void bk_hotkeys_init(bk_hotkey_cb cb) {
    g_hotkey_cb = cb;
    if (g_hotkeys) return;
    g_hotkeys = [NSMutableDictionary new];
    EventTypeSpec spec = {kEventClassKeyboard, kEventHotKeyPressed};
    InstallApplicationEventHandler(&hotkey_handler, 1, &spec, NULL, NULL);
}

/// `mods`: Carbon modifiers (cmdKey, shiftKey, optionKey, controlKey).
int bk_hotkey_register(uint32_t id, uint32_t keycode, uint32_t mods) {
    EventHotKeyRef ref = NULL;
    EventHotKeyID hid = {'BLNK', id};
    OSStatus st = RegisterEventHotKey(keycode, mods, hid, GetApplicationEventTarget(), 0, &ref);
    if (st != noErr || !ref) return 0;
    g_hotkeys[@(id)] = [NSValue valueWithPointer:ref];
    return 1;
}

void bk_hotkey_unregister(uint32_t id) {
    NSValue *v = g_hotkeys[@(id)];
    if (!v) return;
    UnregisterEventHotKey((EventHotKeyRef)v.pointerValue);
    [g_hotkeys removeObjectForKey:@(id)];
}
