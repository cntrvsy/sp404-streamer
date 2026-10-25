/// Commands dispatched asynchronously from the GUI or device monitor
/// to the single background audio command worker thread.
#[derive(Debug, Clone)]
pub enum AudioCommand {
    /// Route playback stream to a specific sink node name (None = OS default)
    RoutePlayback { sink_name: Option<String> },

    /// Link the SP-404MKII hardware capture ports to the capture stream with retry
    LinkCapture { sp404_node: String },

    /// Disconnect capture stream when SP-404 is disconnected
    UnlinkCapture,

    /// Mute or unmute local monitor output ports
    SetLocalMonitorMute(bool),

    /// Refresh the list of available audio playback sinks
    RefreshSinks,
}
