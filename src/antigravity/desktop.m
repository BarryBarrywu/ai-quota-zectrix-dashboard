#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>

static id attribute(id element, CFStringRef key) {
    CFTypeRef result = NULL;
    if (!element || AXUIElementCopyAttributeValue((__bridge AXUIElementRef)element, key, &result) != kAXErrorSuccess)
        return nil;
    return CFBridgingRelease(result);
}

static NSString *label(id element) {
    for (NSString *key in @[@"AXTitle", @"AXDescription", @"AXValue"]) {
        id value = attribute(element, (__bridge CFStringRef)key);
        if ([value isKindOfClass:NSString.class] && [value length]) return value;
    }
    return nil;
}

static id find(id root, NSString *role, NSString *name, double deadline) {
    NSMutableArray *queue = [NSMutableArray array];
    if (root) [queue addObject:root];
    for (NSUInteger i = 0; i < queue.count && i < 4000; i++) {
        if (NSProcessInfo.processInfo.systemUptime >= deadline) break;
        id node = queue[i];
        NSString *kind = attribute(node, kAXRoleAttribute);
        if ([kind isEqual:role] && [label(node) isEqual:name]) return node;
        if ([kind isEqual:@"AXStaticText"] || [kind isEqual:@"AXTextArea"]) continue;
        NSArray *children = attribute(node, kAXChildrenAttribute);
        if (children) [queue addObjectsFromArray:children];
    }
    return nil;
}

static id waitFor(id root, NSString *role, NSString *name, double deadline) {
    do {
        id node = find(root, role, name, deadline);
        if (node) return node;
        [NSThread sleepForTimeInterval:0.1];
    } while (NSProcessInfo.processInfo.systemUptime < deadline);
    return nil;
}

static BOOL press(id button) {
    return button && AXUIElementPerformAction((__bridge AXUIElementRef)button, kAXPressAction) == kAXErrorSuccess;
}

// The app exposes each group's quota rows as direct siblings of its AXHeading.
static NSArray *quotaRows(id panel, NSString *name, double deadline) {
    id heading = find(panel, @"AXHeading", name, deadline);
    id parent = attribute(heading, kAXParentAttribute);
    NSArray *siblings = attribute(parent, kAXChildrenAttribute);
    NSUInteger start = [siblings indexOfObject:heading ?: NSNull.null];
    if (!siblings || start == NSNotFound) return nil;
    NSMutableArray *rows = [NSMutableArray array];
    for (NSUInteger i = start + 1; i < siblings.count; i++) {
        NSString *role = attribute(siblings[i], kAXRoleAttribute);
        if ([role isEqual:@"AXHeading"]) break;
        if (![role isEqual:@"AXStaticText"]) continue;
        NSString *value = label(siblings[i]);
        if (value && value.length <= 180) [rows addObject:value];
    }
    return rows;
}

int agy_desktop_quota(char *out, size_t capacity, bool interactive) {
    @autoreleasepool {
        if (!AXIsProcessTrusted()) return 0;
        NSRunningApplication *app = [NSRunningApplication runningApplicationsWithBundleIdentifier:@"com.google.antigravity"].firstObject;
        if (!app) return 0;
        id ax = CFBridgingRelease(AXUIElementCreateApplication(app.processIdentifier));
        AXUIElementSetMessagingTimeout((__bridge AXUIElementRef)ax, 0.35);
        AXUIElementSetAttributeValue((__bridge AXUIElementRef)ax, CFSTR("AXManualAccessibility"), kCFBooleanTrue);
        id window = attribute(ax, kAXFocusedWindowAttribute);
        double deadline = NSProcessInfo.processInfo.systemUptime + 7;
        id panel = find(window, @"AXGroup", @"Settings", deadline);
        BOOL opened = NO;
        if (!panel) {
            if ((!interactive && app.active) || !press(find(window, @"AXButton", @"Settings", deadline))) return 0;
            panel = waitFor(window, @"AXGroup", @"Settings", deadline);
            opened = panel != nil;
        }
        @try {
            id refresh = find(panel, @"AXButton", @"Refresh quota and credits data", deadline);
            if (!refresh && opened && press(find(panel, @"AXButton", @"Models", deadline)))
                refresh = waitFor(panel, @"AXButton", @"Refresh quota and credits data", deadline);
            id oldHeading = find(panel, @"AXHeading", @"Gemini Models", deadline);
            if (!press(refresh)) return 0;
            while (oldHeading && [oldHeading isEqual:find(panel, @"AXHeading", @"Gemini Models", deadline)]) {
                if (NSProcessInfo.processInfo.systemUptime >= deadline) return 0;
                [NSThread sleepForTimeInterval:0.05];
            }
            // Refresh removes the old cards while the backend request is pending.
            if (!waitFor(panel, @"AXHeading", @"Gemini Models", deadline)) return 0;
            NSArray *gemini = quotaRows(panel, @"Gemini Models", deadline);
            NSArray *third = quotaRows(panel, @"Claude and GPT models", deadline);
            if (!gemini.count || !third.count) return 0;
            NSData *data = [NSJSONSerialization dataWithJSONObject:@[gemini, third] options:0 error:NULL];
            if (!data || data.length >= capacity) return 0;
            memcpy(out, data.bytes, data.length);
            return (int)data.length;
        } @finally {
            if (opened && (interactive || !app.active))
                press(find(panel, @"AXButton", @"Close", NSProcessInfo.processInfo.systemUptime + 0.5));
        }
    }
}
