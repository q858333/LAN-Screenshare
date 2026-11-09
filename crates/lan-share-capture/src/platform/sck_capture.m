#import <string.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#import <Foundation/Foundation.h>

typedef void (*sck_frame_cb)(
    const uint8_t *bgra,
    uint32_t width,
    uint32_t height,
    size_t stride,
    void *ctx);

@interface LanShareCapture : NSObject <SCStreamOutput>
@property(nonatomic, assign) sck_frame_cb callback;
@property(nonatomic, assign) void *ctx;
@property(nonatomic, strong) SCStream *stream;
@end

@implementation LanShareCapture
- (void)stream:(SCStream *)stream
    didOutputSampleBuffer:(CMSampleBufferRef)sampleBuffer
                   ofType:(SCStreamOutputType)type {
    (void)stream;
    if (type != SCStreamOutputTypeScreen || self.callback == NULL) {
        return;
    }
    CVImageBufferRef image = CMSampleBufferGetImageBuffer(sampleBuffer);
    if (image == NULL) {
        return;
    }
    CVPixelBufferLockBaseAddress(image, kCVPixelBufferLock_ReadOnly);
    uint8_t *base = CVPixelBufferGetBaseAddress(image);
    size_t width = CVPixelBufferGetWidth(image);
    size_t height = CVPixelBufferGetHeight(image);
    size_t stride = CVPixelBufferGetBytesPerRow(image);
    if (base != NULL && width > 0 && height > 0 && stride >= width * 4) {
        self.callback(base, (uint32_t)width, (uint32_t)height, stride, self.ctx);
    }
    CVPixelBufferUnlockBaseAddress(image, kCVPixelBufferLock_ReadOnly);
}
@end

static void copy_error(char *err, size_t err_len, NSString *message) {
    if (err == NULL || err_len == 0) {
        return;
    }
    NSString *text = message ?: @"屏幕采集失败";
    const char *utf8 = text.UTF8String ?: "屏幕采集失败";
    strlcpy(err, utf8, err_len);
}

void *sck_start(
    uint32_t fps,
    sck_frame_cb callback,
    void *ctx,
    uint32_t *out_width,
    uint32_t *out_height,
    char *err,
    size_t err_len) {
    if (fps == 0) {
        fps = 30;
    }
    dispatch_semaphore_t ready = dispatch_semaphore_create(0);
    __block SCShareableContent *content = nil;
    __block NSError *share_error = nil;
    [SCShareableContent getShareableContentWithCompletionHandler:^(SCShareableContent *result, NSError *error) {
        content = result;
        share_error = error;
        dispatch_semaphore_signal(ready);
    }];
    if (dispatch_semaphore_wait(ready, dispatch_time(DISPATCH_TIME_NOW, 8 * NSEC_PER_SEC)) != 0) {
        copy_error(err, err_len, @"等待屏幕采集权限超时。请在系统设置 → 隐私与安全性 → 屏幕录制中允许本应用。");
        return NULL;
    }
    if (content == nil || content.displays.count == 0) {
        copy_error(err, err_len, share_error.localizedDescription ?: @"没有可用的显示器");
        return NULL;
    }

    SCDisplay *display = content.displays.firstObject;
    SCContentFilter *filter = [[SCContentFilter alloc] initWithDisplay:display excludingWindows:@[]];
    SCStreamConfiguration *config = [SCStreamConfiguration new];
    config.width = (size_t)display.width;
    config.height = (size_t)display.height;
    config.pixelFormat = kCVPixelFormatType_32BGRA;
    config.showsCursor = YES;
    config.minimumFrameInterval = CMTimeMake(1, (int32_t)fps);

    LanShareCapture *capture = [LanShareCapture new];
    capture.callback = callback;
    capture.ctx = ctx;
    NSError *output_error = nil;
    SCStream *stream = [[SCStream alloc] initWithFilter:filter configuration:config delegate:nil];
    if (![stream addStreamOutput:capture type:SCStreamOutputTypeScreen sampleHandlerQueue:nil error:&output_error]) {
        copy_error(err, err_len, output_error.localizedDescription ?: @"无法注册画面回调");
        return NULL;
    }

    dispatch_semaphore_t started = dispatch_semaphore_create(0);
    __block NSError *start_error = nil;
    [stream startCaptureWithCompletionHandler:^(NSError *error) {
        start_error = error;
        dispatch_semaphore_signal(started);
    }];
    if (dispatch_semaphore_wait(started, dispatch_time(DISPATCH_TIME_NOW, 8 * NSEC_PER_SEC)) != 0) {
        copy_error(err, err_len, @"启动屏幕采集超时。请允许屏幕录制后重新点「启动分享」。");
        return NULL;
    }
    if (start_error != nil) {
        copy_error(err, err_len, start_error.localizedDescription);
        return NULL;
    }
    capture.stream = stream;
    if (out_width != NULL) {
        *out_width = (uint32_t)config.width;
    }
    if (out_height != NULL) {
        *out_height = (uint32_t)config.height;
    }
    return (__bridge_retained void *)capture;
}

void sck_stop(void *handle) {
    if (handle == NULL) {
        return;
    }
    LanShareCapture *capture = (__bridge_transfer LanShareCapture *)handle;
    capture.callback = NULL;
    SCStream *stream = capture.stream;
    if (stream == nil) {
        return;
    }
    dispatch_semaphore_t stopped = dispatch_semaphore_create(0);
    [stream stopCaptureWithCompletionHandler:^(NSError *error) {
        (void)error;
        dispatch_semaphore_signal(stopped);
    }];
    dispatch_semaphore_wait(stopped, dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC));
}
