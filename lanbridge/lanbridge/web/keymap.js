// W3C KeyboardEvent.code validation table matching lanbridge/protocol.py
(function (global) {
  "use strict";

  const SUPPORTED_CODES = new Set([
    "Escape", "Digit1", "Digit2", "Digit3", "Digit4", "Digit5", "Digit6",
    "Digit7", "Digit8", "Digit9", "Digit0", "Minus", "Equal", "Backspace",
    "Tab", "KeyQ", "KeyW", "KeyE", "KeyR", "KeyT", "KeyY", "KeyU", "KeyI",
    "KeyO", "KeyP", "BracketLeft", "BracketRight", "Enter", "ControlLeft",
    "KeyA", "KeyS", "KeyD", "KeyF", "KeyG", "KeyH", "KeyJ", "KeyK", "KeyL",
    "Semicolon", "Quote", "Backquote", "ShiftLeft", "Backslash", "KeyZ",
    "KeyX", "KeyC", "KeyV", "KeyB", "KeyN", "KeyM", "Comma", "Period",
    "Slash", "ShiftRight", "NumpadMultiply", "AltLeft", "Space", "CapsLock",
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "NumLock",
    "ScrollLock", "Numpad7", "Numpad8", "Numpad9", "NumpadSubtract", "Numpad4",
    "Numpad5", "Numpad6", "NumpadAdd", "Numpad1", "Numpad2", "Numpad3",
    "Numpad0", "NumpadDecimal", "IntlBackslash", "F11", "F12", "NumpadEnter",
    "ControlRight", "NumpadDivide", "PrintScreen", "AltRight", "Home",
    "ArrowUp", "PageUp", "ArrowLeft", "ArrowRight", "End", "ArrowDown",
    "PageDown", "Insert", "Delete", "AudioVolumeMute", "AudioVolumeDown",
    "AudioVolumeUp", "Pause", "NumpadComma", "MetaLeft", "MetaRight",
    "ContextMenu", "MediaTrackNext", "MediaPlayPause", "MediaTrackPrevious",
    "MediaStop", "F13", "F14", "F15", "F16", "F17", "F18", "F19", "F20",
    "F21", "F22", "F23", "F24"
  ]);

  global.LanBridgeKeymap = {
    isSupportedCode: (code) => SUPPORTED_CODES.has(code)
  };
})(window);
