# Third-party notices

Release installers bundle unmodified command-line builds of FFmpeg and FFprobe from
[BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds). Those binaries and their
source are available under the licenses listed by that project, including the GNU GPL.
Clip Engine invokes them as separate processes and does not link to FFmpeg libraries.

The desktop UI embeds Latin subsets of [IBM Plex Sans](https://github.com/IBM/plex) and
IBM Plex Mono, licensed under the SIL Open Font License.

Desktop builds also link dynamically against [libmpv](https://mpv.io/) for playback.
libmpv is available under GNU GPL/LGPL terms from the mpv project. Windows installers
include `mpv-2.dll`; Linux packages ship `libmpv`.

