import QtQuick
import qs.Commons

Text {
  property string label: ""
  property color foreground: Color.foreground
  property string fontFamily: Style.font.family
  textFormat: Text.PlainText
  text: "[ " + label.toUpperCase() + " ]"
  color: Color.muted
  font.family: fontFamily
  font.pixelSize: Style.font.caption
  font.bold: true
  topPadding: Math.ceil(Style.font.caption * 0.15)
  // Match the compact section labels used by the Kanagawa Omarchy modules
  // without depending on a user's theme directory.
}
