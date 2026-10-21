// Recording chrome: borderless, non-activating panels that show a picture
// drawn by Rust (the dashed frame, the timer pill), plus a main-thread timer.

#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>
#include "bk.h"

typedef void (*bk_panel_cb)(void *ctx, int event, double x, double y);  // 1 move, 2 leave, 3 click (points in the panel)

@interface BKPanelView : NSView
@property(nonatomic) bk_panel_cb cb;
@property(nonatomic) void *ctx;
@property(nonatomic) BOOL hand;
@end

@implementation BKPanelView
- (BOOL)isFlipped {
    return YES;
}
- (BOOL)acceptsFirstMouse:(NSEvent *)e {
    (void)e;
    return YES;
}
- (void)updateTrackingAreas {
    [super updateTrackingAreas];
    for (NSTrackingArea *a in self.trackingAreas) [self removeTrackingArea:a];
    [self addTrackingArea:[[NSTrackingArea alloc] initWithRect:self.bounds
                                                       options:NSTrackingMouseMoved | NSTrackingMouseEnteredAndExited |
                                                               NSTrackingActiveAlways | NSTrackingCursorUpdate | NSTrackingInVisibleRect
                                                         owner:self
                                                      userInfo:nil]];
}
- (void)report:(int)ev with:(NSEvent *)e {
    NSPoint p = [self convertPoint:e.locationInWindow fromView:nil];
    if (self.cb) self.cb(self.ctx, ev, p.x, p.y);
}
- (void)mouseMoved:(NSEvent *)e {
    [self report:1 with:e];
    [(self.hand ? NSCursor.pointingHandCursor : NSCursor.arrowCursor) set];
}
- (void)mouseEntered:(NSEvent *)e {
    [self report:1 with:e];
}
- (void)mouseExited:(NSEvent *)e {
    [self report:2 with:e];
    [NSCursor.arrowCursor set];
}
- (void)cursorUpdate:(NSEvent *)e {
    (void)e;
    [(self.hand ? NSCursor.pointingHandCursor : NSCursor.arrowCursor) set];
}
- (void)mouseDown:(NSEvent *)e {
    (void)e;
}
- (void)mouseUp:(NSEvent *)e {
    [self report:3 with:e];
}
@end

@interface BKPanel : NSPanel
@property(nonatomic) BOOL closing;
@end
@implementation BKPanel
// winit closes every window of the app when one of our egui windows goes
// away; the recording pill and frame must outlive that
- (void)close {
    if (self.closing) [super close];
}
- (BOOL)canBecomeKeyWindow {
    return NO;
}
- (BOOL)canBecomeMainWindow {
    return NO;
}
@end

void *bk_panel_new(int click_through, bk_panel_cb cb, void *ctx) {
    __block BKPanel *p = nil;
    void (^make)(void) = ^{
        p = [[BKPanel alloc] initWithContentRect:NSMakeRect(0, 0, 10, 10)
                                       styleMask:NSWindowStyleMaskBorderless | NSWindowStyleMaskNonactivatingPanel
                                         backing:NSBackingStoreBuffered
                                           defer:NO];
        p.opaque = NO;
        p.backgroundColor = NSColor.clearColor;
        // the pill (and the permission note) get the system shadow
        p.hasShadow = click_through ? NO : YES;
        p.level = NSStatusWindowLevel;
        p.ignoresMouseEvents = click_through ? YES : NO;
        p.hidesOnDeactivate = NO;
        p.releasedWhenClosed = NO;
        p.animationBehavior = NSWindowAnimationBehaviorNone;
        p.sharingType = NSWindowSharingNone;
        p.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces | NSWindowCollectionBehaviorStationary |
                               NSWindowCollectionBehaviorFullScreenAuxiliary | NSWindowCollectionBehaviorIgnoresCycle;
        BKPanelView *v = [[BKPanelView alloc] initWithFrame:NSMakeRect(0, 0, 10, 10)];
        v.wantsLayer = YES;
        v.layer.contentsGravity = kCAGravityResize;
        v.cb = cb;
        v.ctx = ctx;
        p.contentView = v;
    };
    if ([NSThread isMainThread]) make();
    else dispatch_sync(dispatch_get_main_queue(), make);
    return (__bridge_retained void *)p;
}

/// New picture (premultiplied RGBA, `w × h` px) at (x, y) top-left global
/// points; the panel is `w / scale` points wide.
void bk_panel_set_image(void *pp, const uint8_t *rgba, int w, int h, double x, double y, double scale) {
    BKPanel *p = (__bridge BKPanel *)pp;
    CGColorSpaceRef cs = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    CFDataRef data = CFDataCreate(NULL, rgba, (CFIndex)w * h * 4);
    CGDataProviderRef prov = CGDataProviderCreateWithCFData(data);
    CGImageRef img = CGImageCreate((size_t)w, (size_t)h, 8, 32, (size_t)w * 4, cs, kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big, prov,
                                   NULL, false, kCGRenderingIntentDefault);
    CGDataProviderRelease(prov);
    CFRelease(data);
    CGColorSpaceRelease(cs);
    bk_on_main(^{
        double s = scale > 0 ? scale : 1;
        double pw = w / s, ph = h / s;
        double h0 = NSScreen.screens.count ? NSScreen.screens[0].frame.size.height : 0;
        NSRect f = NSMakeRect(x, h0 - (y + ph), pw, ph);
        BOOL resized = !NSEqualSizes(p.frame.size, f.size);
        if (!NSEqualRects(p.frame, f)) [p setFrame:f display:NO];
        [CATransaction begin];
        [CATransaction setDisableActions:YES];
        p.contentView.layer.contents = (__bridge id)img;
        p.contentView.layer.contentsScale = s;
        [CATransaction commit];
        CGImageRelease(img);
        if (resized && p.hasShadow) [p invalidateShadow];
    });
}

/// Moves the panel (top-left in global points) without redrawing it.
void bk_panel_move(void *pp, double x, double y) {
    BKPanel *p = (__bridge BKPanel *)pp;
    bk_on_main(^{
        double h0 = NSScreen.screens.count ? NSScreen.screens[0].frame.size.height : 0;
        NSPoint o = NSMakePoint(x, h0 - (y + p.frame.size.height));
        if (!NSEqualPoints(p.frame.origin, o)) [p setFrameOrigin:o];
    });
}

void bk_panel_set_hand(void *pp, int hand) {
    BKPanel *p = (__bridge BKPanel *)pp;
    bk_on_main(^{
        BKPanelView *v = (BKPanelView *)p.contentView;
        if (v.hand != (hand != 0)) {
            v.hand = hand != 0;
            [(v.hand ? NSCursor.pointingHandCursor : NSCursor.arrowCursor) set];
        }
    });
}

void bk_panel_show(void *pp) {
    BKPanel *p = (__bridge BKPanel *)pp;
    bk_on_main(^{
        [p orderFrontRegardless];
    });
}

void bk_panel_close(void *pp) {
    if (!pp) return;
    BKPanel *p = (__bridge_transfer BKPanel *)pp;
    bk_on_main(^{
        ((BKPanelView *)p.contentView).cb = NULL;
        p.closing = YES;
        [p orderOut:nil];
        [p close];
    });
}

// ---------------------------------------------------------------------------
// Timer on the main run loop (also while menus are open)

typedef void (*bk_timer_cb)(void *ctx);

@interface BKTimerTarget : NSObject
@property(nonatomic) bk_timer_cb cb;
@property(nonatomic) void *ctx;
@end
@implementation BKTimerTarget
- (void)fire:(id)t {
    (void)t;
    if (self.cb) self.cb(self.ctx);
}
@end

void *bk_timer_start(double interval, bk_timer_cb cb, void *ctx) {
    BKTimerTarget *target = [BKTimerTarget new];
    target.cb = cb;
    target.ctx = ctx;
    NSTimer *t = [NSTimer timerWithTimeInterval:interval target:target selector:@selector(fire:) userInfo:nil repeats:YES];
    bk_on_main(^{
        [NSRunLoop.mainRunLoop addTimer:t forMode:NSRunLoopCommonModes];
    });
    return (__bridge_retained void *)t;
}

void bk_timer_stop(void *p) {
    if (!p) return;
    id t = (__bridge_transfer id)p;
    bk_on_main(^{
        [t invalidate];
    });
}

/// Like `bk_timer_start`, but once per frame of the display `panel` is on
/// (120 times a second on ProMotion): for following a moving window.
void *bk_frame_timer_start(void *panel, bk_timer_cb cb, void *ctx) {
    BKPanel *p = (__bridge BKPanel *)panel;
    BKTimerTarget *target = [BKTimerTarget new];
    target.cb = cb;
    target.ctx = ctx;
    __block CADisplayLink *link = nil;
    void (^make)(void) = ^{
        link = [p.contentView displayLinkWithTarget:target selector:@selector(fire:)];
        [link addToRunLoop:NSRunLoop.mainRunLoop forMode:NSRunLoopCommonModes];
    };
    if ([NSThread isMainThread]) make();
    else dispatch_sync(dispatch_get_main_queue(), make);
    return (__bridge_retained void *)link;
}
