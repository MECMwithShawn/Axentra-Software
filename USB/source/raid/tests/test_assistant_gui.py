"""Optional display smoke test: run under xvfb-run with python3-tk installed."""
import os
import unittest
from unittest.mock import patch
from test_assistant import a, fixture
try:
    import tkinter as tk
    from tkinter import ttk
except ImportError:
    tk = None


@unittest.skipUnless(tk and os.environ.get('DISPLAY'), 'requires tkinter and a display (e.g. Xvfb)')
class GuiTests(unittest.TestCase):
    def test_review_plan_and_cancel_finalization_without_privileged_commands(self):
        snap = fixture()
        snap['devices']['/dev/md42'].update(size=100*1024**3, fstype=None)
        def inspect(window):
            window.update()
            def widgets(parent):
                for child in parent.winfo_children():
                    yield child
                    yield from widgets(child)
            all_widgets = list(widgets(window))
            boxes = [w for w in all_widgets if isinstance(w, ttk.Combobox)]
            for box in boxes:
                self.assertEqual(box.get(), '')
                box.current(0)
            buttons = {w.cget('text'): w for w in all_widgets if isinstance(w, ttk.Button)}
            for button in buttons.values():
                self.assertTrue(button.winfo_viewable())
                self.assertLess(button.winfo_rooty() + button.winfo_height(), window.winfo_rooty() + window.winfo_height() + 1)
            buttons['Review plan'].invoke()
            text = next(w for w in all_widgets if isinstance(w, tk.Text))
            self.assertIn('PERSONALIZED INSTALLATION PLAN', text.get('1.0', 'end'))
            with patch('tkinter.messagebox.askyesno', return_value=False):
                buttons['Finalize installed system'].invoke()
            buttons['Refresh disks'].invoke()
            self.assertTrue(all(box.get() == '' for box in boxes))
            window.destroy()
        with patch.object(a, 'inventory', return_value=snap), patch.object(a.subprocess, 'Popen') as process, patch.object(tk.Tk, 'mainloop', inspect):
            a.gui()
            process.assert_not_called()
