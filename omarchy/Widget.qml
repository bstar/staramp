import QtQuick
import QtQuick.Controls
import Quickshell
import Quickshell.Io
import Quickshell.Services.Mpris
import qs.Commons
import qs.Ui

BarWidget {
  id: root
  moduleName: "bstar.staramp"
  readonly property string pluginVersion: "0.1.1"

  readonly property var players: Mpris.players ? Mpris.players.values : []
  readonly property var player: {
    for (var i = 0; i < players.length; ++i) {
      if (players[i] && players[i].dbusName === "org.mpris.MediaPlayer2.staramp") return players[i]
    }
    return null
  }
  readonly property string title: player ? String(player.trackTitle || "") : ""
  readonly property string artist: player ? String(player.trackArtist || "") : ""
  readonly property bool canPlayPause: !!player && (player.canTogglePlaying || player.canPlay || player.canPause)
  readonly property bool canScrub: !!player && player.canSeek && player.positionSupported
    && player.lengthSupported && player.length > 0
  readonly property color ink: bar ? bar.foreground : Color.foreground
  readonly property string face: bar ? bar.fontFamily : Style.font.family
  property bool popupOpen: false
  property var queueStatus: null
  property string observedVersion: ""
  property bool statusFailed: false
  property var listeningSnapshot: null
  readonly property bool opened: popupOpen
  property bool popoutSwitchClosing: false
  // WidgetButton.labelWidth includes font bearings; mark the painted label.
  readonly property real openPanelIndicatorWidth: player && !vertical
    ? Math.max(1, Math.ceil(titleMetrics.tightBoundingRect(button.text).width))
    : button.labelWidth
  readonly property bool versionMismatch: observedVersion !== "" && observedVersion !== pluginVersion

  function close() { popupOpen = false }
  function open() { popoutSwitchClosing = false; popupOpen = true }
  function closeForPopoutSwitch() {
    popoutSwitchClosing = true
    close()
  }
  function playPause() {
    if (!player || !canPlayPause) return
    if (player.canTogglePlaying) player.togglePlaying()
    else if (player.isPlaying && player.canPause) player.pause()
    else if (!player.isPlaying && player.canPlay) player.play()
  }
  function clock(seconds) {
    var n = Math.max(0, Math.floor(Number(seconds) || 0))
    return Math.floor(n / 60) + ":" + String(n % 60).padStart(2, "0")
  }
  function queueMatches(status) {
    if (!player || !status || !player.metadata) return false
    var id = player.metadata["mpris:trackid"]
    return !versionMismatch && id !== undefined
      && String(id).indexOf(String(status.trackId)) !== -1
      && root.title === String(status.title || "")
      && root.artist === String(status.artist || "")
  }
  function refreshQueue() {
    if (popupOpen && player && !statusFailed && !statusProcess.running) statusProcess.running = true
  }
  function refreshListening() {
    if (popupOpen && !listeningProcess.running) listeningProcess.running = true
  }

  onPlayerChanged: {
    queueStatus = null
    observedVersion = ""
    statusFailed = false
    if (player) refreshQueue()
  }
  onPopupOpenChanged: {
    if (popupOpen) {
      popoutSwitchClosing = false
      statusFailed = false
      refreshQueue()
      refreshListening()
    } else {
      queueStatus = null
      statusProcess.running = false
      listeningProcess.running = false
    }
  }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  FontMetrics {
    id: titleMetrics
    font.family: button.fontFamily
    font.pixelSize: button.fontSize
  }

  IpcHandler {
    target: "bstar.staramp"
    function open(): void { root.open() }
    function close(): void { root.close() }
    function toggle(): void { root.popupOpen = !root.popupOpen }
  }

  WidgetButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: !root.player ? "󰝚" : root.vertical ? (root.player.isPlaying ? "󰐊" : "󰏤")
      : (root.player.isPlaying ? "󰐊  " : "󰏤  ") + (root.title || "STAR/AMP")
    fontSize: Style.font.body
    horizontalMargin: 6
    tooltipText: root.player ? (root.title || "STAR/AMP") + (root.artist ? " — " + root.artist : "") : "STAR/AMP is not running"
    onPressed: function(b) {
      if (b === Qt.MiddleButton) root.playPause()
      else if (b === Qt.RightButton && root.player && root.player.canGoNext) root.player.next()
      else if (b === Qt.LeftButton) root.popupOpen = !root.popupOpen
    }

  }

  // The CLI supplies only queue context. MPRIS owns all transport and slider state.
  Process {
    id: statusProcess
    command: ["staramp", "ctl", "status"]
    onExited: function(exitCode) {
      if (exitCode !== 0) root.statusFailed = true
    }
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        if (!root.popupOpen || !root.player) return
        try {
          var next = JSON.parse(String(text))
          root.observedVersion = String(next.version || "older")
          root.queueStatus = root.queueMatches(next) ? next : null
        } catch (e) {
          root.queueStatus = null
        }
      }
    }
  }
  Process {
    id: listeningProcess
    command: ["staramp", "scrobble", "snapshot"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        if (!root.popupOpen) return
        try {
          var next = JSON.parse(String(text))
          root.listeningSnapshot = next.version === root.pluginVersion ? next : null
        } catch (e) {
          root.listeningSnapshot = null
        }
      }
    }
  }
  Timer {
    interval: 30000
    repeat: true
    running: root.popupOpen
    onTriggered: root.refreshListening()
  }
  Timer {
    interval: 2000
    repeat: true
    running: root.popupOpen && !!root.player
    onTriggered: root.refreshQueue()
  }
  Timer {
    interval: 1000
    repeat: true
    running: root.popupOpen && !!root.player && root.player.isPlaying
    onTriggered: root.player.positionChanged()
  }

  AnimatedKeyboardPanel {
    id: popup
    anchorItem: root
    bar: root.bar
    owner: root
    open: root.popupOpen
    focusTarget: keyCatcher
    padding: Style.space(10)
    contentWidth: popup.fittedContentWidth(Style.space(430))
    contentHeight: popup.fittedContentHeight(content.implicitHeight, Style.space(680))

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onCloseRequested: root.close()
      onTabRequested: function(direction) {
        if (root.bar && root.bar.switchPanelFrom) root.bar.switchPanelFrom(root, direction)
      }

    ScrollView {
      id: scrollArea
      anchors.fill: parent
      clip: true
      ScrollBar.horizontal.policy: ScrollBar.AlwaysOff
      ScrollBar.vertical.policy: content.implicitHeight > height ? ScrollBar.AsNeeded : ScrollBar.AlwaysOff

    Column {
      id: content
      width: scrollArea.availableWidth
      spacing: Style.space(8)

      SectionHeader { label: "STAR/AMP · NOW PLAYING"; foreground: root.ink; fontFamily: root.face }

      Row {
        width: parent.width
        spacing: Style.space(10)
        BorderSurface {
          width: Style.space(108)
          height: width
          radius: 0
          color: Style.normalFillFor(root.ink, Color.accent)
          borderSpec: Border.controlSpec("normal", root.ink, Color.accent)
          Image {
            anchors.fill: parent
            anchors.margins: Style.space(2)
            fillMode: Image.PreserveAspectCrop
            asynchronous: true
            source: root.player ? root.player.trackArtUrl : ""
          }
          Text {
            anchors.centerIn: parent
            visible: !root.player || !root.player.trackArtUrl
            text: "󰝚"
            color: root.ink
            font.family: root.face
            font.pixelSize: Style.font.displayLarge
          }
        }
        Column {
          width: parent.width - Style.space(118)
          spacing: Style.space(4)
          Text {
            width: parent.width
            textFormat: Text.PlainText
            text: !root.player ? "STAR/AMP is not running" : root.title || "Nothing playing"
            color: root.ink
            font.family: root.face
            font.pixelSize: Style.font.subtitle
            font.bold: true
            elide: Text.ElideRight
          }
          Text {
            width: parent.width
            visible: text !== ""
            textFormat: Text.PlainText
            text: root.artist
            color: root.ink
            font.family: root.face
            font.pixelSize: Style.font.bodySmall
            elide: Text.ElideRight
          }
          Text {
            width: parent.width
            visible: text !== ""
            textFormat: Text.PlainText
            text: root.player ? root.player.trackAlbum || "" : ""
            color: Qt.darker(root.ink, 1.3)
            font.family: root.face
            font.pixelSize: Style.font.caption
            elide: Text.ElideRight
          }
          Text {
            visible: root.queueMatches(root.queueStatus) && root.queueStatus.total > 0
            text: visible ? "Track " + root.queueStatus.index + " of " + root.queueStatus.total : ""
            color: Qt.darker(root.ink, 1.3)
            font.family: root.face
            font.pixelSize: Style.font.caption
          }
          Text {
            width: parent.width
            visible: root.versionMismatch
            text: "STAR/AMP and widget versions differ"
            color: root.ink
            font.family: root.face
            font.pixelSize: Style.font.caption
            elide: Text.ElideRight
          }
        }
      }

      Row {
        width: parent.width
        spacing: Style.space(8)
        visible: root.player && root.player.lengthSupported
        Text {
          id: elapsedTime
          text: root.player ? root.clock(root.player.position) : "0:00"
          color: root.ink
          font.family: root.face
          font.pixelSize: Style.font.caption
          anchors.verticalCenter: parent.verticalCenter
        }
        PanelSlider {
          width: Math.max(1, parent.width - elapsedTime.implicitWidth - durationTime.implicitWidth - parent.spacing * 2)
          bar: root.bar
          enabled: root.canScrub
          opacity: enabled ? 1 : 0.4
          maximum: root.player && root.player.lengthSupported ? Math.max(1, root.player.length) : 1
          value: root.player ? root.player.position : 0
          onReleased: function(value) {
            if (root.canScrub) root.player.position = value
          }
        }
        Text {
          id: durationTime
          text: root.player ? root.clock(root.player.length) : "0:00"
          color: root.ink
          font.family: root.face
          font.pixelSize: Style.font.caption
          anchors.verticalCenter: parent.verticalCenter
        }
      }

      Row {
        width: parent.width
        spacing: Style.space(6)
        Row {
          id: transportControls
          spacing: Style.space(2)
          anchors.verticalCenter: parent.verticalCenter
        Button {
          iconText: "󰒟"
          horizontalPadding: Style.space(4)
          verticalPadding: Style.space(4)
          tooltipText: "Shuffle"
          focusable: true
          foreground: root.ink
          enabled: !!root.player && root.player.shuffleSupported && root.player.canControl
          selected: enabled && root.player.shuffle
          onClicked: root.player.shuffle = !root.player.shuffle
        }
        Button {
          iconText: "󰒮"
          horizontalPadding: Style.space(4)
          verticalPadding: Style.space(4)
          tooltipText: "Previous track"
          focusable: true
          foreground: root.ink
          enabled: root.player && root.player.canGoPrevious
          onClicked: root.player.previous()
        }
        Button {
          iconText: root.player && root.player.isPlaying ? "󰏤" : "󰐊"
          horizontalPadding: Style.space(4)
          verticalPadding: Style.space(4)
          tooltipText: root.player && root.player.isPlaying ? "Pause" : "Play"
          focusable: true
          foreground: root.ink
          enabled: root.canPlayPause
          onClicked: root.playPause()
        }
        Button {
          iconText: "󰒭"
          horizontalPadding: Style.space(4)
          verticalPadding: Style.space(4)
          tooltipText: "Next track"
          focusable: true
          foreground: root.ink
          enabled: root.player && root.player.canGoNext
          onClicked: root.player.next()
        }
        Button {
          iconText: "󰓛"
          horizontalPadding: Style.space(4)
          verticalPadding: Style.space(4)
          tooltipText: "Stop"
          focusable: true
          foreground: root.ink
          enabled: !!root.player && root.player.canControl
          onClicked: root.player.stop()
        }
        Button {
          iconText: root.player && root.player.loopState === MprisLoopState.Track ? "󰑘" : "󰑖"
          horizontalPadding: Style.space(4)
          verticalPadding: Style.space(4)
          tooltipText: "Repeat mode"
          focusable: true
          foreground: root.ink
          enabled: !!root.player && root.player.loopSupported && root.player.canControl
            && root.observedVersion === root.pluginVersion
          selected: enabled && root.player.loopState !== MprisLoopState.None
          onClicked: {
            if (!root.player) return
            if (root.player.loopState === MprisLoopState.None) root.player.loopState = MprisLoopState.Playlist
            else if (root.player.loopState === MprisLoopState.Playlist) root.player.loopState = MprisLoopState.Track
            else root.player.loopState = MprisLoopState.None
          }
        }
      }
        Text {
          id: volumeLabel
          text: "VOL"
          color: root.ink
          font.family: root.face
          font.pixelSize: Style.font.caption
          anchors.verticalCenter: parent.verticalCenter
        }
        PanelSlider {
          width: Math.max(1, parent.width - transportControls.width - volumeLabel.implicitWidth - volumePercent.width - parent.spacing * 3)
          bar: root.bar
          anchors.verticalCenter: parent.verticalCenter
          enabled: !!root.player && root.player.volumeSupported && root.player.canControl
          opacity: enabled ? 1 : 0.4
          value: root.player ? root.player.volume : 0
          onReleased: function(value) {
            if (root.player && root.player.volumeSupported) root.player.volume = value
          }
        }
        Text {
          id: volumePercent
          width: Style.space(36)
          text: root.player && root.player.volumeSupported ? Math.round(root.player.volume * 100) + "%" : "—"
          color: root.ink
          font.family: root.face
          font.pixelSize: Style.font.caption
          anchors.verticalCenter: parent.verticalCenter
          horizontalAlignment: Text.AlignRight
        }
      }

      PanelSeparator { width: parent.width; foreground: root.ink }
      SectionHeader { label: "LISTENING · SCROBBLING"; foreground: root.ink; fontFamily: root.face }

      Repeater {
        model: root.listeningSnapshot && root.listeningSnapshot.providers ? root.listeningSnapshot.providers : []
        delegate: Row {
          required property var modelData
          width: content.width
          spacing: Style.space(8)
          Text {
            width: Style.space(110)
            text: String(parent.modelData.name || "")
            color: root.ink
            font.family: root.face
            font.pixelSize: Style.font.bodySmall
            font.bold: true
          }
          Text {
            width: parent.width - Style.space(118)
            text: !parent.modelData.authenticated ? "Not connected"
              : !parent.modelData.enabled ? "Connected · off"
              : "Scrobbling" + (parent.modelData.username ? " as " + parent.modelData.username : "")
            color: parent.modelData.authenticated && parent.modelData.enabled ? Color.accent : Color.muted
            font.family: root.face
            font.pixelSize: Style.font.bodySmall
            elide: Text.ElideRight
          }
        }
      }
      Text {
        visible: !!root.listeningSnapshot && root.listeningSnapshot.pending > 0
        text: root.listeningSnapshot ? root.listeningSnapshot.pending + " scrobbles waiting to send" : ""
        color: Color.muted
        font.family: root.face
        font.pixelSize: Style.font.caption
      }

      SectionHeader { label: "RECENT LISTENS"; foreground: root.ink; fontFamily: root.face }
      Repeater {
        model: root.listeningSnapshot && root.listeningSnapshot.recent ? root.listeningSnapshot.recent : []
        delegate: Item {
          required property var modelData
          required property int index
          width: content.width
          height: Style.space(34)
          Text {
            width: Style.space(25)
            anchors.verticalCenter: parent.verticalCenter
            text: String(index + 1).padStart(2, "0")
            color: Color.accent
            font.family: root.face
            font.pixelSize: Style.font.caption
            font.bold: true
          }
          Column {
            x: Style.space(30)
            width: parent.width - Style.space(100)
            anchors.verticalCenter: parent.verticalCenter
            Text {
              width: parent.width
              text: String(modelData.title || "Unknown track")
              color: root.ink
              font.family: root.face
              font.pixelSize: Style.font.bodySmall
              font.bold: true
              elide: Text.ElideRight
            }
            Text {
              width: parent.width
              text: String(modelData.artist || "Unknown artist")
                + (modelData.album ? " · " + modelData.album : "")
              color: Color.muted
              font.family: root.face
              font.pixelSize: Style.font.caption
              elide: Text.ElideRight
            }
          }
          Text {
            width: Style.space(65)
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            horizontalAlignment: Text.AlignRight
            text: String(modelData.state || "")
            color: modelData.state === "sent" ? Color.accent : Color.muted
            font.family: root.face
            font.pixelSize: Style.font.caption
          }
        }
      }
      Text {
        visible: !root.listeningSnapshot || !root.listeningSnapshot.recent || root.listeningSnapshot.recent.length === 0
        text: root.listeningSnapshot ? "No listening history yet" : "Listening history appears after STAR/AMP restarts"
        color: Color.muted
        font.family: root.face
        font.pixelSize: Style.font.caption
      }
    }
    }
    }
  }
}
