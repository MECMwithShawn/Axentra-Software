// Opens the Axentra OneDrive panel just above its tray icon, with the
// keyboard focus, as Windows does. On Wayland apps can't place or focus
// their own windows, so the shell does it for this one window.

import Meta from 'gi://Meta';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const APP_ID = 'axentra-onedrive';
const PANEL_TITLE = 'Axentra OneDrive';
// Space between the panel and the taskbar or the screen edge.
const GAP = 12;

export default class AxentraOneDrivePanel extends Extension {
    enable() {
        this._created = global.display.connect('window-created',
            (_display, window) => this._watch(window));
    }

    disable() {
        global.display.disconnect(this._created);
        this._created = 0;
    }

    _watch(window) {
        // On X11 the app places and focuses the panel itself.
        if (window.get_client_type() !== Meta.WindowClientType.WAYLAND)
            return;
        const actor = window.get_compositor_private();
        if (!actor)
            return;
        // The app id and title are known once the first frame is drawn. The
        // panel's first frame is still transparent, so the move isn't seen.
        const id = actor.connect('first-frame', () => {
            actor.disconnect(id);
            if (window.get_wm_class() === APP_ID && window.get_title() === PANEL_TITLE)
                this._place(window);
        });
    }

    _place(window) {
        const [px, py] = global.get_pointer();
        const area = window.get_work_area_for_monitor(global.display.get_current_monitor());
        const frame = window.get_frame_rect();
        // Clicked on the taskbar (outside the work area): next to the click,
        // above it or below it. Opened some other way: the corner by the
        // notification area.
        const onBar = px < area.x || px >= area.x + area.width ||
            py < area.y || py >= area.y + area.height;
        const anchorX = onBar ? px : area.x + area.width;
        const below = onBar ? py >= area.y + area.height / 2 : true;
        const x = Math.max(area.x + GAP,
            Math.min(anchorX - frame.width / 2, area.x + area.width - frame.width - GAP));
        const y = below ? area.y + area.height - frame.height - GAP : area.y + GAP;
        window.move_frame(true, Math.round(x), Math.round(y));
        window.make_above();
        Main.activateWindow(window);
    }
}
