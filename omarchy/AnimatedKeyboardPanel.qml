import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

// Keep this alongside the widget so the plugin works without a user's theme
// components. Kanagawa panels run this sweep when their popouts open.
KeyboardPanel {
  id: root
  property string themeName: ""

  property FileView themeFile: FileView {
    path: Quickshell.env("HOME") + "/.local/state/omarchy/current/theme.name"
    watchChanges: true
    printErrors: false
    onLoaded: root.themeName = text().trim()
    onFileChanged: reload()
  }

  Item {
    id: reveal
    anchors.fill: parent
    z: 100
    enabled: false
    visible: root.themeName === "kanagawa-solid" && (root.open || sweep.running)

    Rectangle {
      id: line
      x: 0
      width: parent.width
      height: 1
      color: Color.accent
      opacity: 0
    }

    SequentialAnimation {
      id: sweep
      PropertyAction { target: line; property: "opacity"; value: 0.75 }
      NumberAnimation { target: line; property: "y"; from: 0; to: reveal.height; duration: 260; easing.type: Easing.InOutQuad }
      NumberAnimation { target: line; property: "opacity"; to: 0; duration: 120 }
    }
  }

  onOpenChanged: if (open && themeName === "kanagawa-solid") sweep.restart()
  onThemeNameChanged: if (open && themeName === "kanagawa-solid") sweep.restart()
}
