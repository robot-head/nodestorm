/* @ts-self-types="./ferroterm_wasm.d.ts" */

/**
 * A terminal instance usable from JavaScript.
 */
export class Terminal {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        TerminalFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_terminal_free(ptr, 0);
    }
    /**
     * @returns {boolean}
     */
    appCursorKeys() {
        const ret = wasm.terminal_appCursorKeys(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {number}
     */
    bellCount() {
        const ret = wasm.terminal_bellCount(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {boolean}
     */
    bracketedPaste() {
        const ret = wasm.terminal_bracketedPaste(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * Encode a printable character press (with Ctrl/Alt folding).
     * @param {number} code_point
     * @param {number} mods
     * @returns {Uint8Array}
     */
    char(code_point, mods) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.terminal_char(retptr, this.__wbg_ptr, code_point, mods);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var v1 = getArrayU8FromWasm0(r0, r1).slice();
            wasm.__wbindgen_export(r0, r1 * 1, 1);
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {number}
     */
    cols() {
        const ret = wasm.terminal_cols(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {boolean}
     */
    cursorVisible() {
        const ret = wasm.terminal_cursorVisible(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {number}
     */
    displayOffset() {
        const ret = wasm.terminal_displayOffset(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Feed raw bytes received from the host / PTY.
     * @param {Uint8Array} bytes
     */
    feed(bytes) {
        const ptr0 = passArray8ToWasm0(bytes, wasm.__wbindgen_export2);
        const len0 = WASM_VECTOR_LEN;
        wasm.terminal_feed(this.__wbg_ptr, ptr0, len0);
    }
    /**
     * Feed a UTF-16 JS string (encoded to UTF-8 on the boundary).
     * @param {string} s
     */
    feedStr(s) {
        const ptr0 = passStringToWasm0(s, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
        const len0 = WASM_VECTOR_LEN;
        wasm.terminal_feedStr(this.__wbg_ptr, ptr0, len0);
    }
    /**
     * Resolve a grapheme-cluster id (from a cell's `grapheme` field in the
     * snapshot) to the full cluster string (base + combining marks, a ZWJ
     * emoji sequence, or a regional-indicator flag).
     * @param {number} id
     * @returns {string | undefined}
     */
    grapheme(id) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.terminal_grapheme(retptr, this.__wbg_ptr, id);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * Raw encoded file bytes of image `id` (iTerm2 OSC 1337 path), empty for a
     * Sixel/RGBA image. The front-end decodes these with `createImageBitmap`.
     * @param {number} id
     * @returns {Uint8Array}
     */
    imageEncoded(id) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.terminal_imageEncoded(retptr, this.__wbg_ptr, id);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var v1 = getArrayU8FromWasm0(r0, r1).slice();
            wasm.__wbindgen_export(r0, r1 * 1, 1);
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * Live image ids (oldest first).
     * @returns {Uint32Array}
     */
    imageIds() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.terminal_imageIds(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var v1 = getArrayU32FromWasm0(r0, r1).slice();
            wasm.__wbindgen_export(r0, r1 * 4, 4);
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * MIME hint for `imageEncoded(id)` (e.g. `image/png`), empty for RGBA.
     * @param {number} id
     * @returns {string}
     */
    imageMime(id) {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.terminal_imageMime(retptr, this.__wbg_ptr, id);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Per-frame placements: flat `[id, viewportRow, col, widthPx, heightPx] …`.
     * @returns {Int32Array}
     */
    imagePlacements() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.terminal_imagePlacements(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var v1 = getArrayI32FromWasm0(r0, r1).slice();
            wasm.__wbindgen_export(r0, r1 * 4, 4);
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * RGBA bytes of image `id` (`width*height*4`), empty if gone.
     * @param {number} id
     * @returns {Uint8Array}
     */
    imageRgba(id) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.terminal_imageRgba(retptr, this.__wbg_ptr, id);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var v1 = getArrayU8FromWasm0(r0, r1).slice();
            wasm.__wbindgen_export(r0, r1 * 1, 1);
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * `[width, height]` in pixels of image `id`.
     * @param {number} id
     * @returns {Uint32Array}
     */
    imageSize(id) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.terminal_imageSize(retptr, this.__wbg_ptr, id);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var v1 = getArrayU32FromWasm0(r0, r1).slice();
            wasm.__wbindgen_export(r0, r1 * 4, 4);
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * Counter bumped when the image set changes; re-sync textures on change.
     * @returns {number}
     */
    imagesVersion() {
        const ret = wasm.terminal_imagesVersion(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Encode a special key press to the bytes a host program expects.
     *
     * `key` is the [`KeyCode`] discriminant; `mods` is a bitmask
     * (1=shift, 2=alt, 4=ctrl, 8=meta). Returns the bytes to send to the PTY.
     * @param {number} key
     * @param {number} mods
     * @returns {Uint8Array}
     */
    key(key, mods) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.terminal_key(retptr, this.__wbg_ptr, key, mods);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var v1 = getArrayU8FromWasm0(r0, r1).slice();
            wasm.__wbindgen_export(r0, r1 * 1, 1);
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @param {number} abs
     * @returns {string}
     */
    lineText(abs) {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.terminal_lineText(retptr, this.__wbg_ptr, abs);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Resolve an OSC 8 hyperlink id (from a cell's `link` field) to its URI.
     * @param {number} id
     * @returns {string | undefined}
     */
    linkUri(id) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.terminal_linkUri(retptr, this.__wbg_ptr, id);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            let v1;
            if (r0 !== 0) {
                v1 = getStringFromWasm0(r0, r1).slice();
                wasm.__wbindgen_export(r0, r1 * 1, 1);
            }
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * Encode a mouse event to the current mouse protocol, or return empty if
     * mouse reporting is off. `button`: 0=left,1=middle,2=right,64=wheel-up,
     * 65=wheel-down. `action`: 0=press,1=release,2=move.
     * @param {number} button
     * @param {number} col
     * @param {number} row
     * @param {number} action
     * @param {number} mods
     * @returns {Uint8Array}
     */
    mouse(button, col, row, action, mods) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.terminal_mouse(retptr, this.__wbg_ptr, button, col, row, action, mods);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var v1 = getArrayU8FromWasm0(r0, r1).slice();
            wasm.__wbindgen_export(r0, r1 * 1, 1);
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {number}
     */
    mouseMode() {
        const ret = wasm.terminal_mouseMode(this.__wbg_ptr);
        return ret;
    }
    /**
     * @returns {boolean}
     */
    mouseSgr() {
        const ret = wasm.terminal_mouseSgr(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * Create a terminal of `cols` x `rows` with `scrollback` lines of history.
     * @param {number} cols
     * @param {number} rows
     * @param {number} scrollback
     */
    constructor(cols, rows, scrollback) {
        const ret = wasm.terminal_new(cols, rows, scrollback);
        this.__wbg_ptr = ret;
        TerminalFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * Current palette overrides: `[fg, bg, cursor, c0..c255]` (259 words), each
     * `0` for "no override" or a packed `0x02_RRGGBB`.
     * @returns {Uint32Array}
     */
    paletteExport() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.terminal_paletteExport(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var v1 = getArrayU32FromWasm0(r0, r1).slice();
            wasm.__wbindgen_export(r0, r1 * 4, 4);
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * Counter bumped whenever the dynamic palette (OSC 4/10/11/12/104…)
     * changes; the front-end re-reads `paletteExport` when it differs.
     * @returns {number}
     */
    paletteVersion() {
        const ret = wasm.terminal_paletteVersion(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Resize the grid.
     * @param {number} cols
     * @param {number} rows
     */
    resize(cols, rows) {
        wasm.terminal_resize(this.__wbg_ptr, cols, rows);
    }
    /**
     * @returns {number}
     */
    rows() {
        const ret = wasm.terminal_rows(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @param {number} delta
     */
    scrollLines(delta) {
        wasm.terminal_scrollLines(this.__wbg_ptr, delta);
    }
    scrollToBottom() {
        wasm.terminal_scrollToBottom(this.__wbg_ptr);
    }
    /**
     * @param {number} abs
     */
    scrollToLine(abs) {
        wasm.terminal_scrollToLine(this.__wbg_ptr, abs);
    }
    /**
     * @returns {number}
     */
    scrollbackLen() {
        const ret = wasm.terminal_scrollbackLen(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Cell size in device pixels, so Sixel images lay out in whole cells.
     * @param {number} w
     * @param {number} h
     */
    setCellPixels(w, h) {
        wasm.terminal_setCellPixels(this.__wbg_ptr, w, h);
    }
    /**
     * Provide the theme's default fg/bg/cursor (packed RGB, low 24 bits) so the
     * core can answer OSC color queries for un-overridden colors.
     * @param {number} fg
     * @param {number} bg
     * @param {number} cursor
     */
    setDefaultColors(fg, bg, cursor) {
        wasm.terminal_setDefaultColors(this.__wbg_ptr, fg, bg, cursor);
    }
    /**
     * Produce a render snapshot as a packed `Uint32Array`.
     * Pass `force = true` to emit every row (e.g. after a theme change).
     *
     * This copies the data across the wasm boundary. The render loop uses the
     * zero-copy [`snapshot_ptr`](Self::snapshot_ptr) path instead; this remains
     * for callers (tests, benchmarks) that want an owned array.
     * @param {boolean} force
     * @returns {Uint32Array}
     */
    snapshot(force) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.terminal_snapshot(retptr, this.__wbg_ptr, force);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var v1 = getArrayU32FromWasm0(r0, r1).slice();
            wasm.__wbindgen_export(r0, r1 * 4, 4);
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * Length (in `u32` words) of the buffer produced by the last
     * [`snapshot_ptr`](Self::snapshot_ptr) call.
     * @returns {number}
     */
    snapshotLen() {
        const ret = wasm.terminal_snapshotLen(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Build the snapshot into the persistent buffer and return a pointer to it
     * in wasm linear memory. JavaScript wraps `[ptr, len]` in a `Uint32Array`
     * view — no allocation, no copy. The pointer is valid until the next call
     * that mutates the terminal (which may reallocate the buffer or grow
     * memory), so the caller must read the view before doing anything else.
     * @param {boolean} force
     * @returns {number}
     */
    snapshotPtr(force) {
        const ret = wasm.terminal_snapshotPtr(this.__wbg_ptr, force);
        return ret >>> 0;
    }
    /**
     * Drain bytes the terminal wants to send back to the host (reply to DSR,
     * device attributes, etc.). Send these to the PTY.
     * @returns {Uint8Array}
     */
    takeOutput() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.terminal_takeOutput(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var v1 = getArrayU8FromWasm0(r0, r1).slice();
            wasm.__wbindgen_export(r0, r1 * 1, 1);
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * The window title (OSC 0/2). Read [`title_changed`] to know when to poll.
     * @returns {string}
     */
    title() {
        let deferred1_0;
        let deferred1_1;
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.terminal_title(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            deferred1_0 = r0;
            deferred1_1 = r1;
            return getStringFromWasm0(r0, r1);
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
            wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * @returns {boolean}
     */
    titleChanged() {
        const ret = wasm.terminal_titleChanged(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @returns {number}
     */
    totalLines() {
        const ret = wasm.terminal_totalLines(this.__wbg_ptr);
        return ret >>> 0;
    }
}
if (Symbol.dispose) Terminal.prototype[Symbol.dispose] = Terminal.prototype.free;
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg___wbindgen_throw_1506f2235d1bdba0: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbg_error_2d54bccfcbfedc7c: function(arg0, arg1) {
            let deferred0_0;
            let deferred0_1;
            try {
                deferred0_0 = arg0;
                deferred0_1 = arg1;
                console.error(getStringFromWasm0(arg0, arg1));
            } finally {
                wasm.__wbindgen_export(deferred0_0, deferred0_1, 1);
            }
        },
    };
    return {
        __proto__: null,
        "./ferroterm_wasm_bg.js": import0,
    };
}

const TerminalFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_terminal_free(ptr, 1));

function getArrayI32FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getInt32ArrayMemory0().subarray(ptr / 4, ptr / 4 + len);
}

function getArrayU32FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint32ArrayMemory0().subarray(ptr / 4, ptr / 4 + len);
}

function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}

let cachedDataViewMemory0 = null;
function getDataViewMemory0() {
    if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
        cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
    }
    return cachedDataViewMemory0;
}

let cachedInt32ArrayMemory0 = null;
function getInt32ArrayMemory0() {
    if (cachedInt32ArrayMemory0 === null || cachedInt32ArrayMemory0.byteLength === 0) {
        cachedInt32ArrayMemory0 = new Int32Array(wasm.memory.buffer);
    }
    return cachedInt32ArrayMemory0;
}

function getStringFromWasm0(ptr, len) {
    return decodeText(ptr >>> 0, len);
}

let cachedUint32ArrayMemory0 = null;
function getUint32ArrayMemory0() {
    if (cachedUint32ArrayMemory0 === null || cachedUint32ArrayMemory0.byteLength === 0) {
        cachedUint32ArrayMemory0 = new Uint32Array(wasm.memory.buffer);
    }
    return cachedUint32ArrayMemory0;
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function passArray8ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 1, 1) >>> 0;
    getUint8ArrayMemory0().set(arg, ptr / 1);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

let wasmModule, wasmInstance, wasm;
function __wbg_finalize_init(instance, module) {
    wasmInstance = instance;
    wasm = instance.exports;
    wasmModule = module;
    cachedDataViewMemory0 = null;
    cachedInt32ArrayMemory0 = null;
    cachedUint32ArrayMemory0 = null;
    cachedUint8ArrayMemory0 = null;
    return wasm;
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = module.ok && expectedResponseType(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else { throw e; }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }

    function expectedResponseType(type) {
        switch (type) {
            case 'basic': case 'cors': case 'default': return true;
        }
        return false;
    }
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (module !== undefined) {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;


    if (module_or_path !== undefined) {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (module_or_path === undefined) {
        module_or_path = new URL('ferroterm_wasm_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync, __wbg_init as default };
