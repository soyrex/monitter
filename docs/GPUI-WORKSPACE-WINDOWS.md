# Workspace windows

The GPUI desktop can move a named workspace into another native window. This
moves its retained UI; it does not launch a second backend or restart agents.

## Controls

- Open a workspace tab's menu and choose **Move to New Window**.
- In a detached window, choose **Return to Main Window** to return that workspace.
- On macOS, drag a workspace tab outside the open Monitter windows and release it
  to create a window. Drop it on another window's tab strip to move it there.
- Closing a detached window returns all its workspaces to the main window.
  Closing the main window follows the existing whole-application close flow.

Workspace switching and movement wait for preference restoration and an existing
preference save to finish. A workspace has one native window owner at a time.
Existing drafts, dock contents, terminal buffers and backend session identities
remain retained. Moving a browser tab reparents its existing native webview on
macOS, preserving its page and history.

Preferences remain available in each window. The shared session-lock controls
open in the main window so password fields have one focus and input owner.

Window placement, workspace ownership and each window's selected workspace are
saved in the device-local `window-state.json`. Shared workspace layouts remain
in the normal preferences. Missing or invalid placement data falls back to a
visible default window. Unknown workspace IDs are removed from local ownership
when the current workspace list is reconciled.

## Platform limits

Native tab dragging currently has a macOS adapter. Other platforms use the menu
actions until their native drag lifecycle and global window coordinates have
been implemented and verified. An unknown drop position never creates a window.
Live browser-view transfer is currently supported on macOS; unsupported native
browser transfers return a visible error and keep the workspace in its source.
Windows and Linux runtime behavior still needs testing on those platforms.

A chat, draft or native browser pane cannot be mounted in two native windows.
If multiple workspace layouts contain that same pane, movement is rejected with
an explanation; remove the duplicate pane from one workspace first. Empty
windows can accept another workspace through their tab strip. Closing a window
or removing a workspace does not terminate its agents or terminal sessions.

Normal application quit waits for restoration and the single preference writer,
captures every retained workspace, and closes all native hosts. A forced system
quit is subject to the operating system and GPUI shutdown deadline; unfinished
restoration or newly dirty state may not finish saving in that situation.
