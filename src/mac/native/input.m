// Clicks and key presses anywhere on screen, for the effects drawn into a
// recording. A listen-only event tap on its own thread; keyboard events need
// the Input Monitoring permission, clicks work without it.

#import <AppKit/AppKit.h>
#include "bk.h"

typedef void (*bk_click_cb)(double x, double y, int right);
typedef void (*bk_key_cb)(uint16_t keycode, int down, uint64_t flags);

typedef struct {
    CFMachPortRef tap;
    CFRunLoopRef loop;
    bk_click_cb click;
    bk_key_cb key;
} bk_tap;

static CGEventRef tap_cb(CGEventTapProxy proxy, CGEventType type, CGEventRef e, void *ud) {
    (void)proxy;
    bk_tap *t = ud;
    if (type == kCGEventTapDisabledByTimeout || type == kCGEventTapDisabledByUserInput) {
        if (t->tap) CGEventTapEnable(t->tap, true);
        return e;
    }
    switch (type) {
        case kCGEventLeftMouseDown:
        case kCGEventOtherMouseDown:
        case kCGEventRightMouseDown: {
            if (!t->click) break;
            CGPoint p = CGEventGetLocation(e);
            t->click(p.x, p.y, type == kCGEventRightMouseDown);
            break;
        }
        case kCGEventKeyDown:
        case kCGEventKeyUp:
            if (t->key && !CGEventGetIntegerValueField(e, kCGKeyboardEventAutorepeat))
                t->key((uint16_t)CGEventGetIntegerValueField(e, kCGKeyboardEventKeycode), type == kCGEventKeyDown, CGEventGetFlags(e));
            break;
        default:
            break;
    }
    return e;
}

/// Starts listening. Returns NULL when no tap could be created.
void *bk_input_start(int clicks, int keys, bk_click_cb click, bk_key_cb key) {
    bk_tap *t = calloc(1, sizeof(bk_tap));
    t->click = clicks ? click : NULL;
    t->key = keys ? key : NULL;
    CGEventMask mouse = CGEventMaskBit(kCGEventLeftMouseDown) | CGEventMaskBit(kCGEventRightMouseDown) | CGEventMaskBit(kCGEventOtherMouseDown);
    CGEventMask kb = CGEventMaskBit(kCGEventKeyDown) | CGEventMaskBit(kCGEventKeyUp);
    CGEventMask mask = (clicks ? mouse : 0) | (keys ? kb : 0);
    t->tap = CGEventTapCreate(kCGSessionEventTap, kCGHeadInsertEventTap, kCGEventTapOptionListenOnly, mask, tap_cb, t);
    if (!t->tap && keys && clicks) {
        // no Input Monitoring permission: clicks only
        t->key = NULL;
        t->tap = CGEventTapCreate(kCGSessionEventTap, kCGHeadInsertEventTap, kCGEventTapOptionListenOnly, mouse, tap_cb, t);
    }
    if (!t->tap) {
        free(t);
        return NULL;
    }
    dispatch_semaphore_t ready = dispatch_semaphore_create(0);
    NSThread *th = [[NSThread alloc] initWithBlock:^{
        t->loop = CFRunLoopGetCurrent();
        CFRunLoopSourceRef src = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, t->tap, 0);
        CFRunLoopAddSource(t->loop, src, kCFRunLoopCommonModes);
        CGEventTapEnable(t->tap, true);
        dispatch_semaphore_signal(ready);
        CFRunLoopRun();
        CFRunLoopRemoveSource(t->loop, src, kCFRunLoopCommonModes);
        CFRelease(src);
    }];
    th.name = @"blinker.input";
    [th start];
    dispatch_semaphore_wait(ready, DISPATCH_TIME_FOREVER);
    return t;
}

void bk_input_stop(void *p) {
    bk_tap *t = p;
    if (!t) return;
    CGEventTapEnable(t->tap, false);
    t->click = NULL;
    t->key = NULL;
    CFRunLoopStop(t->loop);
    CFMachPortInvalidate(t->tap);
    CFRelease(t->tap);
    // the thread may still be leaving its run loop: the struct is leaked on
    // purpose (a few bytes per recording) rather than freed under it
}
