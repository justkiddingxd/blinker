// Native macOS layer of blinker: plain C functions over AppKit, CoreGraphics,
// ScreenCaptureKit and AVFoundation, called from Rust (src/mac/ffi.rs).
//
// Geometry crosses this boundary in global *points* with a top-left origin
// (CoreGraphics' convention: y grows downwards from the top of the primary
// display). Rust maps points to the physical pixels the rest of blinker uses.

#pragma once
#include <stdint.h>
#include <stddef.h>

#ifdef __OBJC__
#import <Foundation/Foundation.h>
#define BK_ERR(buf, len, ...) bk_set_err(buf, len, [NSString stringWithFormat:__VA_ARGS__])
void bk_set_err(char *buf, int len, NSString *msg);
NSString *bk_str(const char *s);
void bk_on_main(dispatch_block_t b);
#endif

typedef struct {
    double x, y, w, h;      // frame
    double vx, vy, vw, vh;  // visible frame (no menu bar / Dock)
    double scale;           // backing scale (2 on Retina)
    double refresh;         // Hz
    double safe_top;        // points the camera housing (notch) takes at the top
    uint32_t id;            // CGDirectDisplayID
} bk_display;

typedef struct {
    double x, y, w, h;
    uint32_t id;  // CGWindowID
    int32_t pid;
    int32_t layer;
    float alpha;
} bk_win;
