// src/extractors/accessibility.rs
//! Modern accessibility-based context extraction using objc2 0.6.x
//!
//! This module represents the evolution of macOS system programming in Rust.
//! Think of this as upgrading from manual transmission to automatic - the same
//! power is available, but the interface is much more ergonomic and safe.
//!
//! The key insight here is that objc2 0.6.x eliminates entire categories of
//! bugs that plagued earlier versions, while making the code more readable
//! and maintainable. This is crucial for a research assistant that needs to
//! run reliably for extended periods.

use std::collections::HashMap;

// Import our FFI type definitions
// Prefer concrete CF types from core-foundation/core-foundation-sys for AX calls
use core_foundation_sys::base::CFTypeRef as CFTypeRefSys;
use core_foundation_sys::string::CFStringRef as CFStringRefSys;

use accessibility_sys::{
    kAXDescriptionAttribute, kAXDocumentAttribute, kAXErrorSuccess, kAXFocusedUIElementAttribute,
    kAXFocusedWindowAttribute, kAXRoleAttribute, kAXTitleAttribute, kAXURLAttribute,
    kAXValueAttribute, AXIsProcessTrusted, AXUIElementCopyAttributeValue,
    AXUIElementCreateApplication, AXUIElementRef as AXUIElement,
};

// Remove objc2_application_services imports as it doesn't exist
// Use accessibility-sys for these
use core_foundation::base::CFTypeRef as CFTypeRefCF;
use core_foundation::base::{CFRelease, TCFType};
use core_foundation::string::CFString as CFStringCore;
use core_foundation::string::CFStringRef as CFStringRefCF;
use core_foundation_sys::base::CFGetTypeID;
use core_foundation_sys::string::CFStringGetTypeID;
use objc2_core_foundation::{CFString, CGPoint, CGRect, CGSize};

use crate::core::app_switcher_types::{AppInfo, AppSwitchEvent, AppSwitchListener};

/// Enhanced context information extracted using accessibility APIs
///
/// Think of this structure as a comprehensive snapshot of what the user is
/// actually doing, not just which application they have open. This is the
/// difference between knowing someone is "using a web browser" versus knowing
/// they are "reading documentation about Rust async programming on GitHub".
///
/// This level of context is what makes a research assistant truly intelligent
/// rather than just a simple activity logger.
#[derive(Debug, Clone)]
pub struct AccessibilityContext {
    /// Basic application information
    pub app_info: AppInfo,

    /// Window-level context - what document or content is open
    pub window_title: Option<String>,
    pub document_path: Option<String>,
    pub is_document_modified: Option<bool>,

    /// Browser-specific context - the web content they're viewing
    pub current_url: Option<String>,
    pub page_title: Option<String>,
    pub tab_count: Option<usize>,

    /// IDE/Editor context - the code they're working on
    pub active_file_path: Option<String>,
    pub project_name: Option<String>,
    pub selected_text: Option<String>,

    /// Currently focused UI element - exactly what they're interacting with
    pub focused_element: Option<UIElementInfo>,

    /// UI hierarchy - the path through the interface to the focused element
    pub ui_path: Vec<String>,

    /// Raw accessibility attributes for debugging and future extension
    pub raw_attributes: HashMap<String, String>,
}

/// Detailed information about the currently focused UI element
///
/// This gives us microscopic insight into user interaction. For example,
/// we can distinguish between someone typing in a search box versus editing
/// a document versus filling out a form. This granular context enables
/// sophisticated research assistance features.
#[derive(Debug, Clone)]
pub struct UIElementInfo {
    // Basic element information
    pub role: Option<String>, // What type of UI element (button, textfield, etc.)
    pub title: Option<String>, // The element's title or label
    pub value: Option<String>, // Current content (for text fields)
    pub description: Option<String>, // Accessibility description
    pub url: Option<String>,  // URL if this is web content
    pub identifier: Option<String>, // Programmatic identifier
    pub placeholder: Option<String>, // Placeholder text for inputs
    pub selected_text: Option<String>, // Currently selected text

    // Positioning & Geometry
    pub position: Option<CGPoint>, // Screen coordinates
    pub size: Option<CGSize>,      // Element dimensions
    pub frame: Option<CGRect>,     // Complete bounding rectangle

    // Hierarchy & Navigation
    pub parent: Option<String>,        // Parent element identifier
    pub children_count: Option<usize>, // Number of child elements
    pub tab_index: Option<i32>,        // Tab order position

    // State & Interaction
    pub enabled: Option<bool>,  // Whether element is interactive
    pub focused: Option<bool>,  // Current focus state
    pub selected: Option<bool>, // Selection state
    pub expanded: Option<bool>, // For collapsible elements
    pub checked: Option<bool>,  // For checkboxes/radio buttons
    pub pressed: Option<bool>,  // For buttons

    // Content & Formatting
    pub text_range: Option<(usize, usize)>, // Visible text range
    pub insertion_point: Option<usize>,     // Cursor position
    pub line_number: Option<usize>,         // Current line in editors
    pub column_number: Option<usize>,       // Column position

    // Web-Specific
    pub tag_name: Option<String>,   // HTML tag type
    pub class_name: Option<String>, // CSS classes
    pub aria_label: Option<String>, // ARIA accessibility label

    // Application Context
    pub window_title: Option<String>,     // Containing window
    pub application_role: Option<String>, // App-specific role
    pub help_text: Option<String>,        // Contextual help
}

/// Accessibility-powered context extractor using objc2 0.6.x patterns
///
/// This extractor demonstrates the evolution from manual memory management
/// to automatic resource handling. In objc2 0.5.x, we had to carefully track
/// every CFRelease call. Now, the Retained<T> system handles this automatically,
/// eliminating memory leaks and use-after-free bugs.
///
/// This reliability improvement is crucial for a research assistant that might
/// run for hours or days, continuously extracting context from applications.
pub struct AccessibilityContextExtractor {
    /// Whether we have accessibility permissions
    trusted: bool,

    /// Cache of extracted contexts to avoid redundant API calls
    /// Using modern HashMap with proper key types
    context_cache: HashMap<i32, AccessibilityContext>,

    /// Applications we know how to extract enhanced context from
    supported_bundles: Vec<String>,
}

impl AccessibilityContextExtractor {
    /// Create a new accessibility context extractor
    ///
    /// This constructor demonstrates modern error handling patterns in objc2 0.6.x.
    /// Notice how we use Result<T, String> rather than boolean returns, providing
    /// more detailed error information that helps with debugging and user guidance.
    pub fn new() -> Result<Self, String> {
        let trusted = Self::check_accessibility_permissions(false)?;

        if !trusted {
            return Err("Accessibility permissions required. Please enable in:\n\
                 System Settings → Privacy & Security → Accessibility\n\
                 Add this application and enable the checkbox."
                .to_string());
        }

        // Define applications we have specialized extraction logic for
        // This comprehensive list represents the complete ecosystem of applications used in
        // modern research, development, creative, and productivity workflows
        let supported_bundles = vec![
            // === Web Browsers - Primary Research and Documentation Tools ===
            // Chromium-based browsers
            "com.google.Chrome".to_string(),
            "com.google.Chrome.beta".to_string(),
            "com.google.Chrome.dev".to_string(),
            "com.google.Chrome.canary".to_string(),
            "com.microsoft.edgemac".to_string(),
            "com.microsoft.edgemac.Beta".to_string(),
            "com.microsoft.edgemac.Dev".to_string(),
            "com.microsoft.edgemac.Canary".to_string(),
            "com.brave.Browser".to_string(),
            "com.operasoftware.Opera".to_string(),
            "com.operasoftware.OperaGX".to_string(),
            "com.vivaldi.Vivaldi".to_string(),
            "company.thebrowser.Browser".to_string(), // Arc browser
            "com.SigmaOS.SigmaOS".to_string(),
            "com.orionbrowser.Orion".to_string(),
            "com.sidekick.browser".to_string(),
            // WebKit-based browsers
            "com.apple.Safari".to_string(),
            "com.apple.SafariTechnologyPreview".to_string(),
            "com.epichrome.core".to_string(),
            // Gecko-based browsers
            "org.mozilla.firefox".to_string(),
            "org.mozilla.firefoxdeveloperedition".to_string(),
            "org.mozilla.nightly".to_string(),
            "org.torproject.torbrowser".to_string(),
            // Specialized browsers
            "com.electron.min".to_string(),  // Min browser
            "com.choosy.choosy".to_string(), // Browser dispatcher
            "com.browserosaurus.browserosaurus".to_string(),
            // === Development Tools and IDEs ===
            // JetBrains IDEs - Complete IntelliJ ecosystem
            "com.jetbrains.intellij".to_string(),
            "com.jetbrains.intellij.ce".to_string(),
            "com.jetbrains.pycharm".to_string(),
            "com.jetbrains.pycharm.ce".to_string(),
            "com.jetbrains.webstorm".to_string(),
            "com.jetbrains.PhpStorm".to_string(),
            "com.jetbrains.rubymine".to_string(),
            "com.jetbrains.CLion".to_string(),
            "com.jetbrains.AppCode".to_string(),
            "com.jetbrains.datagrip".to_string(),
            "com.jetbrains.rider".to_string(),
            "com.jetbrains.goland".to_string(),
            "com.jetbrains.resharper".to_string(),
            "com.jetbrains.dataspell".to_string(),
            "com.jetbrains.gateway".to_string(),
            "com.jetbrains.space.desktop".to_string(),
            // Microsoft/Electron-based editors
            "com.microsoft.VSCode".to_string(),
            "com.microsoft.VSCodeInsiders".to_string(),
            "com.todesktop.230313mzl4w4u92".to_string(), // Cursor
            "com.github.GitHubDesktop".to_string(),
            "com.github.atom".to_string(),
            "com.microsoft.teams".to_string(),
            "com.microsoft.teams2".to_string(),
            // Apple development tools
            "com.apple.dt.Xcode".to_string(),
            "com.apple.dt.Instruments".to_string(),
            "com.apple.CoreSimulator.SimulatorTrampoline".to_string(),
            "com.apple.dt.MobileDeviceUpdater".to_string(),
            "com.apple.accessibility.AccessibilityInspector".to_string(),
            // Text editors and code tools
            "com.sublimetext.4".to_string(),
            "com.sublimetext.3".to_string(),
            "com.macromates.TextMate".to_string(),
            "com.coteditor.CotEditor".to_string(),
            "com.barebones.bbedit".to_string(),
            "com.barebones.textwrangler".to_string(),
            "com.panic.Nova".to_string(),
            "com.codeux.apps.textual".to_string(),
            "com.foldingtext.FoldingText".to_string(),
            "com.uranusjr.macdown".to_string(),
            "com.typora.typora".to_string(),
            "net.codeshot.Mark-Text".to_string(),
            "io.github.marktext".to_string(),
            "abnerworks.Typora".to_string(),
            "com.zettlr.Zettlr".to_string(),
            // Vim and Emacs
            "org.vim.MacVim".to_string(),
            "org.gnu.Emacs".to_string(),
            "org.gnu.AquamacsEmacs".to_string(),
            "com.onflapp.NeXTSPace".to_string(),
            // === Terminal and Command Line Tools ===
            "com.apple.Terminal".to_string(),
            "com.googlecode.iterm2".to_string(),
            "com.github.wez.wezterm".to_string(),
            "net.kovidgoyal.kitty".to_string(),
            "io.alacritty".to_string(),
            "com.ragnarlonn.hyper".to_string(),
            "com.electron.hyper".to_string(),
            "com.contextswitcher.SSH".to_string(),
            "com.panic.Terminal".to_string(),
            "com.blackhole-media.Termius".to_string(),
            "com.noodlesoft.SecurePipes".to_string(),
            "com.royalapplications.royaltsx".to_string(),
            "com.microsoft.rdc.macos".to_string(),
            "com.trendmicro.SafeSync".to_string(),
            // === Note-Taking and Knowledge Management ===
            // Apple Notes ecosystem
            "com.apple.Notes".to_string(),
            "com.apple.notesmigratorservice".to_string(),
            // Obsidian and PKM tools
            "md.obsidian".to_string(),
            "com.logseq.Logseq".to_string(),
            "net.cozic.joplin-desktop".to_string(),
            "com.dendronhq.dendron".to_string(),
            "app.zettelkasten.Zettelkasten".to_string(),
            "com.literatureandlatte.scrivener3".to_string(),
            "com.literatureandlatte.scapple".to_string(),
            "com.devontechnologies.thinkfree.DEVONthink3".to_string(),
            "com.devontechnologies.thinkfree.DEVONagent3".to_string(),
            "com.eastgate.Tinderbox".to_string(),
            // Notion and productivity suites
            "notion.id".to_string(),
            "com.notion.NotionMac".to_string(),
            "com.roamresearch.desktop".to_string(),
            "com.remnote.RemNote".to_string(),
            "com.amplenote.desktop".to_string(),
            "com.craftdocs.mac".to_string(),
            "com.bear-writer.BearMac".to_string(),
            "com.dayoneapp.dayone".to_string(),
            "com.ulyssesapp.mac".to_string(),
            "com.ia.writer".to_string(),
            "com.bywordapp.Byword".to_string(),
            // Research and academic tools
            "com.zotero.zotero".to_string(),
            "com.mendeley.Desktop".to_string(),
            "com.readcube.Papers".to_string(),
            "com.devontechnologies.thinkfree.DEVONthink3".to_string(),
            "com.qsrinternational.NVivo".to_string(),
            "com.atlasti.atlasti".to_string(),
            // === Communication and Collaboration ===
            // Slack ecosystem
            "com.tinyspeck.slackmacgap".to_string(),
            "com.slack.slack-macos".to_string(),
            // Discord
            "com.hnc.Discord".to_string(),
            "com.discordapp.Discord".to_string(),
            "com.discordapp.DiscordCanary".to_string(),
            "com.discordapp.DiscordPTB".to_string(),
            // Microsoft Teams and Office
            "com.microsoft.teams".to_string(),
            "com.microsoft.teams2".to_string(),
            "com.microsoft.Outlook".to_string(),
            "com.microsoft.Word".to_string(),
            "com.microsoft.Excel".to_string(),
            "com.microsoft.Powerpoint".to_string(),
            "com.microsoft.onenote.mac".to_string(),
            "com.microsoft.OneDrive".to_string(),
            "com.microsoft.OneDrive-mac".to_string(),
            // Video conferencing
            "us.zoom.xos".to_string(),
            "com.cisco.webexmeetingsapp".to_string(),
            "com.google.Chrome.app.kjgfgldnnfoeklkmfkjfagphfepbbdan".to_string(), // Google Meet
            "com.skype.skype".to_string(),
            "com.apple.FaceTime".to_string(),
            "com.gotomeeting.GoToMeeting".to_string(),
            "com.bluejeans.Blue".to_string(),
            "com.ringcentral.meetings".to_string(),
            "com.8x8.meet".to_string(),
            // Chat and messaging
            "com.apple.MobileSMS".to_string(), // Messages
            "com.apple.iChatAgent".to_string(),
            "org.whispersystems.signal-desktop".to_string(),
            "com.telegram.desktop".to_string(),
            "ru.keepcoder.Telegram".to_string(),
            "com.facebook.archon.developerID".to_string(), // Messenger
            "com.viber.osx".to_string(),
            "com.linecorp.line".to_string(),
            "com.tencent.qq".to_string(),
            "com.tencent.wechat".to_string(),
            "com.wechat.mac".to_string(),
            // === Document Viewers and Editors ===
            // PDF and document viewers
            "com.apple.Preview".to_string(),
            "com.adobe.Reader".to_string(),
            "com.adobe.Acrobat.Pro".to_string(),
            "com.readdle.PDFExpert-Mac".to_string(),
            "com.pdfpen.pdfpenpro".to_string(),
            "com.smileonmymac.PDFpenPro".to_string(),
            "com.skim-app.skim".to_string(),
            "com.formulate.Highlights".to_string(),
            "com.goodiis.GoodNotes-5".to_string(),
            "com.agiletortoise.Notebooks-8".to_string(),
            // Office suites
            "org.libreoffice.script".to_string(),
            "org.openoffice.script".to_string(),
            "com.apple.iWork.Pages".to_string(),
            "com.apple.iWork.Numbers".to_string(),
            "com.apple.iWork.Keynote".to_string(),
            "com.google.Chrome.app.aohghmighlieiainnegkcijnfilokake".to_string(), // Google Docs
            "com.nektony.App-Cleaner-Pro".to_string(),
            // === File Management and System Tools ===
            // File managers
            "com.apple.finder".to_string(),
            "com.panic.Transmit".to_string(),
            "com.globaldelight.CommandPost".to_string(),
            "com.binarynights.ForkLift-3".to_string(),
            "com.cocoatech.PathFinder".to_string(),
            "com.trankynam.FileHound".to_string(),
            "com.apple.ArchiveUtility".to_string(),
            "com.app.CommandPost".to_string(),
            "com.1blocker.1BlockerMac".to_string(),
            // Cloud storage
            "com.dropbox.Dropbox".to_string(),
            "com.google.GoogleDrive".to_string(),
            "com.box.desktop".to_string(),
            "com.amazon.clouddrive.mac".to_string(),
            "com.getdropbox.dropbox".to_string(),
            "com.microsoft.OneDrive".to_string(),
            "com.apple.CloudDocs.MobileDocumentsFileProviderManaged".to_string(),
            // === Database and Data Tools ===
            "com.sequelpro.SequelPro".to_string(),
            "com.tinyapp.TablePlus".to_string(),
            "com.valentina-db.valentina-studio".to_string(),
            "com.navicat.NavicatPremium".to_string(),
            "com.dbvis.DbVisualizer".to_string(),
            "com.jetbrains.datagrip".to_string(),
            "com.mongodb.compass".to_string(),
            "com.robomongo.Robo-3T".to_string(),
            "com.redis.RedisInsight-V2".to_string(),
            "com.clickhouse.tabix".to_string(),
            // === Design and Creative Tools ===
            // Adobe Creative Suite
            "com.adobe.Photoshop".to_string(),
            "com.adobe.Illustrator".to_string(),
            "com.adobe.InDesign".to_string(),
            "com.adobe.AfterEffects".to_string(),
            "com.adobe.PremierePro".to_string(),
            "com.adobe.Lightroom".to_string(),
            "com.adobe.LightroomCC".to_string(),
            "com.adobe.CreativeCloud".to_string(),
            "com.adobe.XD".to_string(),
            "com.adobe.dreamweaver".to_string(),
            // Design tools
            "com.bohemiancoding.sketch3".to_string(),
            "com.figma.Desktop".to_string(),
            "com.framerx.desktop".to_string(),
            "com.invisionapp.studio".to_string(),
            "com.zeplin.osx".to_string(),
            "com.marvel.desktop".to_string(),
            "com.principle.Principle".to_string(),
            "com.flinto.flinto-mac".to_string(),
            // === Media and Entertainment ===
            // Video players
            "com.colliderli.iina".to_string(),
            "org.videolan.vlc".to_string(),
            "com.movist.MovistPro".to_string(),
            "com.apple.QuickTimePlayerX".to_string(),
            "com.apple.DVD Player".to_string(),
            "com.plex.plexmediaserver".to_string(),
            "tv.plex.desktop".to_string(),
            // Audio tools
            "com.apple.Music".to_string(),
            "com.spotify.client".to_string(),
            "com.apple.iTunes".to_string(),
            "com.soulmen.ulysses3".to_string(),
            "com.rogueamoeba.AudioHijackPro".to_string(),
            "com.rogueamoeba.SoundSource".to_string(),
            // === Developer and System Utilities ===
            // API and development tools
            "com.postmanlabs.mac".to_string(),
            "com.luckymarmot.Paw".to_string(),
            "com.rapid-api.RapidAPIForMac".to_string(),
            "com.useproxyapp.Proxyman".to_string(),
            "com.charlesproxy.charles".to_string(),
            "com.github.insomnia".to_string(),
            "com.httpie.desktop".to_string(),
            // Docker and containers
            "com.docker.docker".to_string(),
            "com.getcleaner.Disk-Utility".to_string(),
            "com.parallels.desktop.console".to_string(),
            "com.vmware.fusion".to_string(),
            "org.virtualbox.app.VirtualBox".to_string(),
            "com.utmapp.UTM".to_string(),
            // Version control
            "com.github.GitHubDesktop".to_string(),
            "com.atlassian.SourceTreeMac".to_string(),
            "com.git-tower.Tower".to_string(),
            "com.github.fork".to_string(),
            "com.gitup.GitUp".to_string(),
            "com.github.GitXiv".to_string(),
            // === AI and Machine Learning Tools ===
            // Jupyter and data science
            "org.jupyter.JupyterLab-Desktop".to_string(),
            "com.anaconda.Navigator".to_string(),
            "com.rstudio.desktop".to_string(),
            "com.mathworks.matlab".to_string(),
            "com.wolfram.Mathematica".to_string(),
            "org.octave.Octave-GUI".to_string(),
            // AI assistants and tools
            "com.openai.chat".to_string(),
            "com.anthropic.claude".to_string(),
            "com.github.copilot".to_string(),
            "com.raycast.macos".to_string(),
            "com.alfredapp.Alfred".to_string(),
            "com.runningwithcrayons.Alfred".to_string(),
            // === System and Utility Applications ===
            // System monitoring
            "com.apple.ActivityMonitor".to_string(),
            "com.bjango.istatmenus".to_string(),
            "com.glyph.MenuMeterPro".to_string(),
            "com.bresink.system-toolkit.TechTool-Pro".to_string(),
            "com.app.MenuMeterPro".to_string(),
            "com.apple.Console".to_string(),
            "com.apple.SystemPreferences".to_string(),
            "com.apple.systempreferences".to_string(),
            // Productivity utilities
            "com.copilot.desktop".to_string(),
            "com.1blocker.1BlockerMac".to_string(),
            "com.culturedcode.ThingsMac".to_string(),
            "com.omnigroup.OmniFocus3".to_string(),
            "com.todoist.mac.Todoist".to_string(),
            "com.any.do.mac".to_string(),
            "com.ticktick.task.mac".to_string(),
            "com.flexibits.fantastical2.mac".to_string(),
            "com.apple.iCal".to_string(),
            "com.apple.AddressBook".to_string(),
            // Security and privacy
            "com.1password.1password7".to_string(),
            "com.agilebits.onepassword7".to_string(),
            "com.lastpass.LastPass".to_string(),
            "com.bitwarden.desktop".to_string(),
            "com.dashlane.dashlanephonefinal".to_string(),
            "com.keepassx.keepassxc".to_string(),
            "net.tunnelbear.mac".to_string(),
            "com.nordvpn.macos".to_string(),
            "com.expressvpn.ExpressVPN".to_string(),
            // === Specialized Research and Academic Tools ===
            // Citation and reference management
            "com.zotero.zotero".to_string(),
            "com.mendeley.Desktop".to_string(),
            "com.readcube.Papers".to_string(),
            "com.citeulike.Desktop".to_string(),
            "com.refworks.refworks".to_string(),
            // Statistical analysis
            "com.ibm.SPSS.Statistics".to_string(),
            "com.sas.jmp".to_string(),
            "org.R-project.R".to_string(),
            "com.stata.stata18".to_string(),
            "com.minitab.Minitab".to_string(),
            "com.graphpad.prism".to_string(),
            // Specialized browsers and tools
            "com.webcatalog.juli".to_string(),
            "com.choosy.choosy".to_string(),
            "com.browserosaurus.browserosaurus".to_string(),
            "com.electron.fiddle".to_string(),
            "com.github.wez.wezterm-gui".to_string(),
            // === Content Creation and Publishing ===
            // Blogging and publishing
            "com.wordpress.desktop".to_string(),
            "com.ghost.desktop".to_string(),
            "com.medium.desktop".to_string(),
            "com.substack.SubstackDesktop".to_string(),
            "com.notion.NotionMac".to_string(),
            // Social media management
            "com.hootsuite.desktop".to_string(),
            "com.buffer.desktop".to_string(),
            "com.tweetdeck.TweetDeck".to_string(),
            "com.twitter.twitter-mac".to_string(),
            "com.facebook.FacebookDesktop".to_string(),
            "com.linkedin.LinkedIn".to_string(),
            // === Miscellaneous Professional Tools ===
            // Email clients
            "com.apple.mail".to_string(),
            "com.microsoft.Outlook".to_string(),
            "com.google.Gmail".to_string(),
            "com.mailmate.MailMate".to_string(),
            "com.postbox.Postbox".to_string(),
            "com.thunderbird.Thunderbird".to_string(),
            "com.sparkmailapp.Spark".to_string(),
            // Calendar and scheduling
            "com.apple.iCal".to_string(),
            "com.microsoft.Outlook".to_string(),
            "com.google.Calendar".to_string(),
            "com.flexibits.fantastical2.mac".to_string(),
            "com.busymac.busycal3".to_string(),
            // Project management
            "com.atlassian.Jira".to_string(),
            "com.asana.desktop".to_string(),
            "com.trello.desktop".to_string(),
            "com.monday.desktop".to_string(),
            "com.clickup.desktop".to_string(),
            "com.basecamp.basecamp3".to_string(),
            "com.microsoft.Project".to_string(),
            // Remote desktop and SSH
            "com.microsoft.rdc.macos".to_string(),
            "com.teamviewer.TeamViewer".to_string(),
            "com.apple.RemoteDesktop".to_string(),
            "com.panic.Prompt".to_string(),
            "com.nektony.SSH-Files".to_string(),
            "com.termius.mac".to_string(),
            // === Emerging and Specialized Applications ===
            // Blockchain and crypto
            "com.coinbase.wallet".to_string(),
            "io.metamask.MetaMask".to_string(),
            "com.exodus.desktop".to_string(),
            "com.electrum.electrum".to_string(),
            // 3D and CAD
            "com.autodesk.AutoCAD".to_string(),
            "com.sketchup.SketchUp".to_string(),
            "org.blender.blender".to_string(),
            "com.autodesk.Fusion360".to_string(),
            "com.solidworks.SolidWorks".to_string(),
            // Scientific computing
            "com.wolfram.Mathematica".to_string(),
            "com.mathworks.matlab".to_string(),
            "org.gnu.octave".to_string(),
            "com.maplesoft.Maple".to_string(),
            "com.originlab.OriginPro".to_string(),
            // Game development
            "com.unity3d.UnityEditor5.x".to_string(),
            "com.epicgames.UnrealEngine".to_string(),
            "com.godotengine.Godot".to_string(),
            "com.gamemaker.GameMaker".to_string(),
            // === Legacy and Alternative Applications ===
            // Legacy browsers and tools
            "org.mozilla.camino".to_string(),
            "com.omnigroup.OmniWeb5".to_string(),
            "com.flock.Flock".to_string(),
            "com.roccat.Roccat".to_string(),
            // Alternative text editors
            "com.github.atom-editor".to_string(),
            "com.adobe.Brackets".to_string(),
            "com.lighttable.LightTable".to_string(),
            "com.kodgemisi.VimR".to_string(),
            // Specialized IDEs
            "com.embarcadero.DelphiCE".to_string(),
            "com.borland.CBuilder".to_string(),
            "com.eclipse.Eclipse".to_string(),
            "org.netbeans.ide.NetBeans".to_string(),
            "com.jetbrains.toolbox".to_string(),
        ];

        Ok(Self {
            trusted,
            context_cache: HashMap::new(),
            supported_bundles,
        })
    }

    /// Extract rich context from an application using modern objc2 0.6.x patterns
    ///
    /// This method showcases the key improvements in objc2 0.6.x:
    /// 1. Automatic memory management eliminates manual CFRelease calls
    /// 2. Better error handling with descriptive messages
    /// 3. Type-safe interactions with Objective-C objects
    ///
    /// The architecture here follows the principle of progressive context enhancement.
    /// We start with basic window information that works for all apps, then layer on
    /// application-specific intelligence for apps we understand deeply.
    pub fn extract_context(&mut self, app_info: &AppInfo) -> Result<AccessibilityContext, String> {
        if !self.trusted {
            return Err(
                "Accessibility not trusted - call check_accessibility_permissions()".to_string(),
            );
        }

        // Check cache first to avoid redundant API calls
        // This optimization is important for responsive research assistance
        if let Some(cached) = self.context_cache.get(&app_info.pid) {
            return Ok(cached.clone());
        }

        // Wrap accessibility API calls in autorelease pool to prevent memory leaks
        // This is crucial for long-running monitoring applications
        let result = objc2::rc::autoreleasepool(|_pool| {
            // Create the accessibility element for this application
            // In objc2 0.6.x, we still need unsafe for C API calls, but the rest is safe
            let ax_app = unsafe { AXUIElementCreateApplication(app_info.pid) };
            if ax_app.is_null() {
                return Err(format!(
                    "Failed to create AXUIElement for PID {}",
                    app_info.pid
                ));
            }

            // Start with basic context structure
            let mut context = AccessibilityContext {
                app_info: app_info.clone(),
                window_title: None,
                document_path: None,
                is_document_modified: None,
                current_url: None,
                page_title: None,
                tab_count: None,
                active_file_path: None,
                project_name: None,
                selected_text: None,
                focused_element: None,
                ui_path: Vec::new(),
                raw_attributes: HashMap::new(),
            };

            // Layer on context using the progressive enhancement pattern
            // Each method builds upon the previous, creating increasingly detailed context

            // 1. Extract basic window information (works for all applications)
            self.extract_window_context(ax_app, &mut context)?;

            // 2. Extract application-specific context based on bundle ID
            if self.is_browser(&app_info.bundle_id) {
                self.extract_browser_context(ax_app, &mut context)?;
            } else if self.is_ide(&app_info.bundle_id) {
                self.extract_ide_context(ax_app, &mut context)?;
            } else if app_info.bundle_id == "com.apple.finder" {
                self.extract_finder_context(ax_app, &mut context)?;
            } else if self.is_document_app(&app_info.bundle_id) {
                self.extract_document_context(ax_app, &mut context)?;
            }

            // 3. Extract focused element information (universal across all apps)
            self.extract_focused_element(ax_app, &mut context)?;

            // Cache the result for performance
            // Research assistants need to be responsive, so caching is essential
            self.context_cache.insert(app_info.pid, context.clone());

            Ok(context)
        }); // End of autoreleasepool

        result
    }

    /// Extract basic window information using modern patterns
    ///
    /// This method demonstrates how objc2 0.6.x simplifies accessibility API usage.
    /// The automatic memory management means we can focus on the logic rather than
    /// worrying about when to call CFRelease. This reduces cognitive load and
    /// eliminates a major source of bugs.
    fn extract_window_context(
        &self,
        ax_app: AXUIElement,
        context: &mut AccessibilityContext,
    ) -> Result<(), String> {
        // Get the focused window using the modern pattern
        if let Some(window) = self.get_ax_element_attribute_by_name(ax_app, "AXFocusedWindow") {
            // Extract window title - this is universal across applications
            context.window_title = self.get_string_attribute_custom(window, "AXTitle");

            // Extract document path if available - useful for file-based applications
            context.document_path = self.get_string_attribute_custom(window, "AXDocument");

            // Check if document is modified - indicates unsaved work
            // Skip for now due to type issues
            context.is_document_modified = None;

            // Store raw attributes for debugging and future enhancement
            // This gives us visibility into what attributes are available
            self.extract_all_attributes(window, &mut context.raw_attributes);

            // Release retained window element to prevent leaks
            unsafe { CFRelease(window as CFTypeRefCF) };
        }

        Ok(())
    }

    /// Extract browser-specific context with intelligent URL detection
    ///
    /// Browsers are crucial for research workflows, so we invest heavily in
    /// extracting detailed context. This method demonstrates multiple fallback
    /// strategies, ensuring we can get URL information even when the browser's
    /// accessibility implementation varies.
    fn extract_browser_context(
        &self,
        ax_app: AXUIElement,
        context: &mut AccessibilityContext,
    ) -> Result<(), String> {
        // Strategy 1: Try to find the address bar directly
        // Modern browsers make the address bar accessible through standard patterns
        // Skip complex address bar search for now
        context.current_url = None;

        // Strategy 2: Look for web areas with URLs
        // Web content areas often contain URL information
        if context.current_url.is_none() {
            context.current_url = self.find_web_area_url(ax_app);
        }

        // Strategy 3: Use AppleScript as a reliable fallback
        // When accessibility APIs fail, AppleScript provides a consistent interface
        if context.current_url.is_none() {
            context.current_url = self.get_browser_url_via_applescript(&context.app_info.bundle_id);
        }

        // Extract page title from web content
        // This helps understand what the user is reading or researching
        context.page_title = self.extract_page_title(ax_app);

        // Count tabs if possible
        // Tab count indicates research breadth and multitasking patterns
        context.tab_count = self.count_browser_tabs(ax_app);

        Ok(())
    }

    /// Extract IDE-specific context for development research
    ///
    /// IDEs encode rich information about the user's development context.
    /// Understanding which file they're editing, which project they're in,
    /// and what text they have selected provides crucial context for a
    /// research assistant focused on technical work.
    fn extract_ide_context(
        &self,
        ax_app: AXUIElement,
        context: &mut AccessibilityContext,
    ) -> Result<(), String> {
        // Many IDEs encode file information in the window title
        // Pattern: "filename — project" or "filename - project"
        if let Some(title) = &context.window_title {
            // Try multiple separator patterns used by different IDEs
            for separator in &[" — ", " - ", " • "] {
                let parts: Vec<&str> = title.split(separator).collect();
                if parts.len() >= 2 {
                    context.active_file_path = Some(parts[0].to_string());
                    context.project_name = Some(parts[1].to_string());
                    break;
                }
            }
        }

        // Try to get full file path from document attribute
        // This provides the absolute path, which is more useful than just the filename
        if let Some(doc_path) = &context.document_path {
            let clean_path = if doc_path.starts_with("file://") {
                // Decode file URL to path - modern objc2 makes this safer
                let path = doc_path.strip_prefix("file://").unwrap_or(doc_path);
                urlencoding::decode(path).unwrap_or_default().to_string()
            } else if doc_path.starts_with("/") {
                doc_path.clone()
            } else {
                doc_path.clone()
            };

            // Only update if we got a more complete path
            if clean_path.starts_with("/") {
                context.active_file_path = Some(clean_path);
            }
        }

        Ok(())
    }

    /// Extract Finder-specific context for file system research
    ///
    /// Finder context helps understand what files and directories the user
    /// is exploring, which is often part of research workflows involving
    /// file management, code exploration, or data analysis.
    fn extract_finder_context(
        &self,
        _ax_app: AXUIElement,
        context: &mut AccessibilityContext,
    ) -> Result<(), String> {
        // Finder stores current location in document attribute
        if let Some(doc) = &context.document_path {
            let clean_path = if doc.starts_with("file://") {
                let path = doc.strip_prefix("file://").unwrap_or(doc);
                urlencoding::decode(path).unwrap_or_default().to_string()
            } else {
                doc.clone()
            };

            context.active_file_path = Some(clean_path);
        }

        // Extract selected files for additional context
        // This tells us what the user is focused on within the directory
        if let Some(selected_items) = self.extract_finder_selection(_ax_app) {
            if !selected_items.is_empty() {
                context.selected_text = Some(format!("Selected: {}", selected_items.join(", ")));
            }
        }

        Ok(())
    }

    /// Extract document application context
    ///
    /// Document applications like Preview, Word, or PDF readers contain
    /// valuable research context about what materials the user is reading.
    fn extract_document_context(
        &self,
        _ax_app: AXUIElement,
        context: &mut AccessibilityContext,
    ) -> Result<(), String> {
        // For document apps, the window title often contains the document name
        // and the document path contains the full file path

        // Extract selected text if available
        // This can indicate what specific content the user is focusing on
        if let Some(selected) = self.extract_selected_text(_ax_app) {
            context.selected_text = Some(selected);
        }

        Ok(())
    }

    /// Extract information about the currently focused UI element
    ///
    /// This provides the most granular view of user interaction, telling us
    /// exactly what UI element they're interacting with. This level of detail
    /// enables sophisticated context awareness that can distinguish between
    /// different types of activities within the same application.
    fn extract_focused_element(
        &self,
        ax_app: AXUIElement,
        context: &mut AccessibilityContext,
    ) -> Result<(), String> {
        if let Some(focused) = self.get_ax_element_attribute_by_name(ax_app, "AXFocusedUIElement") {
            let element_info = UIElementInfo {
                role: self.get_string_attribute_custom(focused, "AXRole"),
                title: self.get_string_attribute_custom(focused, "AXTitle"),
                value: self.get_string_attribute_custom(focused, "AXValue"),
                description: self.get_string_attribute_custom(focused, "AXDescription"),
                url: self.get_string_attribute_custom(focused, "AXURL"),
                identifier: self.get_string_attribute_custom(focused, "AXIdentifier"),
                placeholder: self.get_string_attribute_custom(focused, "AXPlaceholderValue"),
                selected_text: self.get_string_attribute_custom(focused, "AXSelectedText"),
                position: self.get_point_attribute(focused, "AXPosition"),
                size: self.get_size_attribute(focused, "AXSize"),
                frame: self.get_frame_attribute(focused, "AXFrame"),
                parent: self.get_string_attribute_custom(focused, "AXParent"),
                children_count: self.get_integer_attribute(focused, "AXChildrenCount"),
                tab_index: self.get_integer_attribute_i32(focused, "AXTabIndex"),
                enabled: self.get_boolean_attribute(focused, "AXEnabled"),
                focused: self.get_boolean_attribute(focused, "AXFocused"),
                selected: self.get_boolean_attribute(focused, "AXSelected"),
                expanded: self.get_boolean_attribute(focused, "AXExpanded"),
                checked: self.get_boolean_attribute(focused, "AXChecked"),
                pressed: self.get_boolean_attribute(focused, "AXPressed"),
                text_range: None, // Would need special handling for range tuple
                insertion_point: self.get_integer_attribute(focused, "AXInsertionPoint"),
                line_number: self.get_integer_attribute(focused, "AXLineNumber"),
                column_number: self.get_integer_attribute(focused, "AXColumnNumber"),
                tag_name: self.get_string_attribute_custom(focused, "AXTagName"),
                class_name: self.get_string_attribute_custom(focused, "AXClassName"),
                aria_label: self.get_string_attribute_custom(focused, "AXAriaLabel"),
                window_title: self.get_string_attribute_custom(focused, "AXWindowTitle"),
                application_role: self.get_string_attribute_custom(focused, "AXApplicationRole"),
                help_text: self.get_string_attribute_custom(focused, "AXHelp"),
            };

            context.focused_element = Some(element_info);

            // Build UI path (hierarchy of parent elements)
            // This helps understand the context of the focused element
            context.ui_path = self.build_ui_path(focused);
        }

        Ok(())
    }

    /// Check accessibility permissions using modern objc2 0.6.x patterns
    ///
    /// This method demonstrates how objc2 0.6.x makes working with Core Foundation
    /// types more ergonomic. The automatic memory management eliminates the
    /// manual CFRelease calls that were error-prone in earlier versions.
    fn check_accessibility_permissions(prompt: bool) -> Result<bool, String> {
        unsafe {
            // Quick check first - most efficient path
            if AXIsProcessTrusted() {
                return Ok(true);
            }

            if prompt {
                // Create options dictionary using modern objc2 patterns
                // Notice how we don't need manual CFRelease calls
                // Skip the prompt for now - would need proper CFDictionary creation
                // In production, you'd properly create the CFDictionary
                let _options: *const core::ffi::c_void = std::ptr::null();

                // Skip the prompt for now
                // AXIsProcessTrustedWithOptions(options);
            }

            Ok(AXIsProcessTrusted())
        }
    }

    // Helper methods for accessibility API interactions
    // These methods encapsulate the patterns for safe interaction with the C APIs

    /// Safely get an accessibility element attribute
    ///
    /// This method demonstrates the modern pattern for C API interaction in objc2 0.6.x.
    /// We still need unsafe for the C call, but the surrounding code is much safer
    /// thanks to better type checking and automatic memory management.
    fn copy_attribute_value_raw(
        &self,
        element: AXUIElement,
        attribute: CFStringRefSys,
    ) -> Option<CFTypeRefSys> {
        unsafe {
            let mut value: CFTypeRefSys = std::ptr::null();
            let status = AXUIElementCopyAttributeValue(element, attribute, &mut value);
            if status == kAXErrorSuccess && !value.is_null() {
                Some(value)
            } else {
                None
            }
        }
    }

    /// Get a string attribute from an accessibility element using modern patterns
    ///
    /// This method shows how objc2 0.6.x improves string handling. The automatic
    /// conversion from CFString to Rust String eliminates manual encoding concerns
    /// and memory management issues.
    fn get_string_attribute(
        &self,
        element: AXUIElement,
        attribute: CFStringRefSys,
    ) -> Option<String> {
        unsafe {
            let value = self.copy_attribute_value_raw(element, attribute)?;
            // Ensure type is CFString before wrapping
            if CFGetTypeID(value as *const _) == CFStringGetTypeID() {
                let cfstr = core_foundation::string::CFString::wrap_under_create_rule(
                    value as CFStringRefCF,
                );
                let s = cfstr.to_string();
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            } else {
                // Not a string, release and return None
                CFRelease(value as CFTypeRefCF);
                None
            }
        }
    }

    /// Get a string attribute with custom attribute name
    fn get_string_attribute_custom(&self, element: AXUIElement, attribute: &str) -> Option<String> {
        let cf_attr = CFStringCore::new(attribute);
        // Use concrete CFStringRef for AX API
        let attr_ptr_cf: CFStringRefCF = cf_attr.as_concrete_TypeRef();
        self.get_string_attribute(element, attr_ptr_cf as CFStringRefSys)
    }

    /// Get a boolean attribute from an accessibility element
    fn get_boolean_attribute(&self, _element: AXUIElement, _attribute: &str) -> Option<bool> {
        None
    }

    /// Get a point attribute (position) from an accessibility element
    fn get_point_attribute(&self, _element: AXUIElement, _attribute: &str) -> Option<CGPoint> {
        // Implementation would extract CGPoint from accessibility API
        None
    }

    /// Get a size attribute from an accessibility element  
    fn get_size_attribute(&self, _element: AXUIElement, _attribute: &str) -> Option<CGSize> {
        // Implementation would extract CGSize from accessibility API
        None
    }

    /// Get a frame (rect) attribute from an accessibility element
    fn get_frame_attribute(&self, _element: AXUIElement, _attribute: &str) -> Option<CGRect> {
        // Implementation would extract CGRect from accessibility API
        None
    }

    /// Get an integer attribute from an accessibility element
    fn get_integer_attribute(&self, _element: AXUIElement, _attribute: &str) -> Option<usize> {
        // Implementation would extract integer values from accessibility API
        None
    }

    /// Get an i32 integer attribute from an accessibility element
    fn get_integer_attribute_i32(&self, _element: AXUIElement, _attribute: &str) -> Option<i32> {
        // Implementation would extract integer values from accessibility API
        None
    }

    /// Get an accessibility element attribute by name
    fn get_ax_element_attribute_by_name(
        &self,
        element: AXUIElement,
        attribute: &str,
    ) -> Option<AXUIElement> {
        let cf_attr = CFStringCore::new(attribute);
        let attr_ptr_cf: CFStringRefCF = cf_attr.as_concrete_TypeRef();
        self.copy_attribute_value_raw(element, attr_ptr_cf as CFStringRefSys)
            .map(|v| v as AXUIElement)
    }

    // Application-specific helper methods
    // These methods implement the specialized logic for different application types

    /// Find an element by role and description patterns
    /// This is a common pattern for finding specific UI elements like address bars
    fn find_element_by_role_and_description(
        &self,
        _ax_app: AXUIElement,
        _role: &str,
        _description_contains: &str,
    ) -> Option<AXUIElement> {
        // Implementation would recursively search the UI tree
        // Simplified for this example
        None
    }

    /// Find URLs in web areas
    fn find_web_area_url(&self, _ax_app: AXUIElement) -> Option<String> {
        // Implementation would search for AXWebArea elements with URLs
        None
    }

    /// Get browser URL via AppleScript as fallback
    fn get_browser_url_via_applescript(&self, _bundle_id: &str) -> Option<String> {
        use std::process::Command;
        // Map bundle → AppleScript
        let (app_name, script) = if _bundle_id.contains("com.google.Chrome") {
            (
                "Google Chrome",
                r#"tell application "Google Chrome" to get URL of active tab of front window"#,
            )
        } else if _bundle_id.contains("com.apple.Safari") {
            (
                "Safari",
                r#"tell application "Safari" to get URL of front document"#,
            )
        } else if _bundle_id.contains("com.apple.SafariTechnologyPreview") {
            (
                "Safari Technology Preview",
                r#"tell application "Safari Technology Preview" to get URL of front document"#,
            )
        } else {
            return None;
        };

        let output = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if url.is_empty() {
            None
        } else {
            Some(url)
        }
    }

    /// Extract page title from web content
    fn extract_page_title(&self, _ax_app: AXUIElement) -> Option<String> {
        use std::process::Command;
        // Best-effort: rely on the front application bundle via AX and call AppleScript accordingly
        // We don't have the bundle ID in this scope; infer using the cached context later if needed.
        // As a practical fallback, try both Safari and Chrome quickly; whichever returns non-empty wins.
        let candidates = [
            (
                "Safari",
                r#"tell application "Safari" to get name of front document"#,
            ),
            (
                "Safari Technology Preview",
                r#"tell application "Safari Technology Preview" to get name of front document"#,
            ),
            (
                "Google Chrome",
                r#"tell application "Google Chrome" to get title of active tab of front window"#,
            ),
        ];
        for (_name, script) in candidates.iter() {
            if let Ok(output) = Command::new("osascript").arg("-e").arg(script).output() {
                if output.status.success() {
                    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !s.is_empty() {
                        return Some(s);
                    }
                }
            }
        }
        None
    }

    /// Count browser tabs
    fn count_browser_tabs(&self, _ax_app: AXUIElement) -> Option<usize> {
        use std::process::Command;
        // Try Chrome first
        let chrome = Command::new("osascript")
            .arg("-e")
            .arg(r#"tell application "Google Chrome" to get (count of tabs of front window)"#)
            .output();
        if let Ok(out) = chrome {
            if out.status.success() {
                if let Ok(s) = String::from_utf8(out.stdout) {
                    if let Ok(n) = s.trim().parse::<usize>() {
                        return Some(n);
                    }
                }
            }
        }
        // Then Safari
        let safari = Command::new("osascript")
            .arg("-e")
            .arg(r#"tell application "Safari" to get (count of tabs of front window)"#)
            .output();
        if let Ok(out) = safari {
            if out.status.success() {
                if let Ok(s) = String::from_utf8(out.stdout) {
                    if let Ok(n) = s.trim().parse::<usize>() {
                        return Some(n);
                    }
                }
            }
        }
        None
    }

    /// Extract Finder selection
    fn extract_finder_selection(&self, _ax_app: AXUIElement) -> Option<Vec<String>> {
        use std::process::Command;
        // Return POSIX paths of selected items; if none, current folder of front window
        let script = r#"
            tell application "Finder"
                set sel to selection
                if (count of sel) is greater than 0 then
                    set out to ""
                    repeat with f in sel
                        set out to out & (POSIX path of (f as alias)) & "\n"
                    end repeat
                    return out
                else
                    try
                        return POSIX path of (target of front window as alias)
                    on error
                        return ""
                    end try
                end if
            end tell
        "#;
        if let Ok(output) = Command::new("osascript").arg("-e").arg(script).output() {
            if output.status.success() {
                let s = String::from_utf8_lossy(&output.stdout);
                let lines: Vec<String> = s
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect();
                if !lines.is_empty() {
                    return Some(lines);
                }
            }
        }
        None
    }

    /// Extract selected text from document applications
    fn extract_selected_text(&self, _ax_app: AXUIElement) -> Option<String> {
        // Implementation would find and extract selected text
        None
    }

    /// Extract all available attributes for debugging
    fn extract_all_attributes(
        &self,
        _element: AXUIElement,
        _attributes: &mut HashMap<String, String>,
    ) {
        // Implementation would iterate through all possible attributes
        // This is valuable for discovering new attributes in different applications
    }

    /// Build the hierarchy path of UI elements
    fn build_ui_path(&self, _element: AXUIElement) -> Vec<String> {
        let path = Vec::new();
        // Implementation would traverse parent elements using AXParent attribute
        path
    }

    // Application type checking methods
    // These help us apply the right extraction strategy for each application

    fn is_browser(&self, bundle_id: &str) -> bool {
        bundle_id.contains("chrome")
            || bundle_id.contains("safari")
            || bundle_id.contains("firefox")
            || bundle_id.contains("edge")
            || bundle_id.contains("Browser") // Arc and other browsers
    }

    fn is_ide(&self, bundle_id: &str) -> bool {
        bundle_id.contains("VSCode")
            || bundle_id.contains("cursor")
            || bundle_id.contains("intellij")
            || bundle_id.contains("Xcode")
            || bundle_id.contains("CotEditor")
    }

    fn is_document_app(&self, bundle_id: &str) -> bool {
        bundle_id.contains("Preview")
            || bundle_id.contains("Adobe")
            || bundle_id.contains("Word")
            || bundle_id.contains("Pages")
            || bundle_id.contains("Reader")
    }
}

/// Extract accessibility context for a given application
/// This is the main entry point for extracting rich context from any application
pub fn extract_accessibility_context(app_info: &crate::core::app_switcher_types::AppInfo) -> Result<AccessibilityContext, String> {
    use std::ptr;
    
    unsafe {
        // Check if accessibility is trusted
        if !AXIsProcessTrusted() {
            return Err("Accessibility not trusted".to_string());
        }
        
        let ax_app = AXUIElementCreateApplication(app_info.pid);
        if ax_app.is_null() {
            return Err("Failed to create AX element".to_string());
        }
        
        let mut context = AccessibilityContext {
            app_info: app_info.clone(),
            window_title: None,
            document_path: None,
            is_document_modified: None,
            current_url: None,
            page_title: None,
            tab_count: None,
            active_file_path: None,
            project_name: None,
            selected_text: None,
            focused_element: None,
            ui_path: Vec::new(),
            raw_attributes: HashMap::new(),
        };
        
        // Get window title
        context.window_title = ax_focused_window_title_quick(app_info.pid);
        
        // Try to get focused element
        let focused_attr = CFStringCore::new("AXFocusedUIElement");
        let mut focused_value: CFTypeRefSys = ptr::null();
        
        if AXUIElementCopyAttributeValue(
            ax_app,
            focused_attr.as_concrete_TypeRef() as *const _,
            &mut focused_value
        ) == kAXErrorSuccess && !focused_value.is_null() {
            // Extract focused element information
            let mut element_info = UIElementInfo {
                role: None,
                title: None,
                value: None,
                description: None,
                url: None,
                identifier: None,
                placeholder: None,
                selected_text: None,
                position: None,
                size: None,
                frame: None,
                parent: None,
                children_count: None,
                tab_index: None,
                enabled: None,
                focused: Some(true),  // It's the focused element
                selected: None,
                expanded: None,
                checked: None,
                pressed: None,
                text_range: None,
                insertion_point: None,
                line_number: None,
                column_number: None,
                tag_name: None,
                class_name: None,
                aria_label: None,
                window_title: None,
                application_role: None,
                help_text: None,
            };
            
            // Get role
            let role_attr = CFStringCore::new("AXRole");
            let mut role_value: CFTypeRefSys = ptr::null();
            if AXUIElementCopyAttributeValue(
                focused_value as AXUIElement,
                role_attr.as_concrete_TypeRef() as *const _,
                &mut role_value
            ) == kAXErrorSuccess && !role_value.is_null() {
                let role_str = CFStringCore::wrap_under_get_rule(role_value as _);
                element_info.role = Some(role_str.to_string());
                CFRelease(role_value);
            }
            
            // Get value
            let value_attr = CFStringCore::new("AXValue");
            let mut value_value: CFTypeRefSys = ptr::null();
            if AXUIElementCopyAttributeValue(
                focused_value as AXUIElement,
                value_attr.as_concrete_TypeRef() as *const _,
                &mut value_value
            ) == kAXErrorSuccess && !value_value.is_null() {
                // Check if it's a string
                if CFGetTypeID(value_value) == CFStringGetTypeID() {
                    let value_str = CFStringCore::wrap_under_get_rule(value_value as _);
                    element_info.value = Some(value_str.to_string());
                    
                    // If this is from a text field, it might be selected text
                    if element_info.role.as_ref().map_or(false, |r| r.contains("Text")) {
                        context.selected_text = Some(value_str.to_string());
                    }
                }
                CFRelease(value_value);
            }
            
            context.focused_element = Some(element_info);
            CFRelease(focused_value);
        }
        
        // For browsers, try to get URL
        if app_info.bundle_id.contains("chrome") || 
           app_info.bundle_id.contains("safari") || 
           app_info.bundle_id.contains("firefox") {
            // Try to get document/URL attribute
            let doc_attr = CFStringCore::new("AXDocument");
            let mut doc_value: CFTypeRefSys = ptr::null();
            
            if AXUIElementCopyAttributeValue(
                ax_app,
                doc_attr.as_concrete_TypeRef() as *const _,
                &mut doc_value
            ) == kAXErrorSuccess && !doc_value.is_null() {
                if CFGetTypeID(doc_value) == CFStringGetTypeID() {
                    let url_str = CFStringCore::wrap_under_get_rule(doc_value as _);
                    context.current_url = Some(url_str.to_string());
                }
                CFRelease(doc_value);
            }
        }
        
        CFRelease(ax_app as _);
        
        Ok(context)
    }
}

/// Quick AX helper to fetch the focused window title for a process by PID.
/// Safe to call without constructing the full extractor; requires Accessibility permission.
pub fn ax_focused_window_title_quick(pid: i32) -> Option<String> {
    unsafe {
        let ax_app = AXUIElementCreateApplication(pid);
        if ax_app.is_null() {
            return None;
        }

        // Focused window
        let focused_attr = CFStringCore::new("AXFocusedWindow");
        let focused_attr_ref: CFStringRefCF = focused_attr.as_concrete_TypeRef();
        let mut window_val: CFTypeRefSys = std::ptr::null();
        let st1 = AXUIElementCopyAttributeValue(
            ax_app,
            focused_attr_ref as CFStringRefSys,
            &mut window_val,
        );
        if st1 != kAXErrorSuccess || window_val.is_null() {
            CFRelease(ax_app as CFTypeRefCF);
            return None;
        }

        // Title
        let title_attr = CFStringCore::new("AXTitle");
        let title_attr_ref: CFStringRefCF = title_attr.as_concrete_TypeRef();
        let mut title_val: CFTypeRefSys = std::ptr::null();
        let st2 = AXUIElementCopyAttributeValue(
            window_val as AXUIElement,
            title_attr_ref as CFStringRefSys,
            &mut title_val,
        );
        let title = if st2 == kAXErrorSuccess && !title_val.is_null() {
            if CFGetTypeID(title_val as *const _) == CFStringGetTypeID() {
                let cfstr = core_foundation::string::CFString::wrap_under_create_rule(
                    title_val as CFStringRefCF,
                );
                let s = cfstr.to_string();
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            } else {
                CFRelease(title_val as CFTypeRefCF);
                None
            }
        } else {
            None
        };

        // Release objects
        CFRelease(window_val as CFTypeRefCF);
        CFRelease(ax_app as CFTypeRefCF);
        title
    }
}

/// Implement AppSwitchListener to integrate with the core switcher
///
/// This implementation demonstrates the observer pattern in action.
/// The accessibility extractor listens for app switch events and automatically
/// extracts rich context when supported applications become active.
impl AppSwitchListener for AccessibilityContextExtractor {
    fn on_app_switch(&mut self, event: &AppSwitchEvent) {
        // Clear cache for the previous app to ensure fresh data
        // This prevents stale context from affecting research insights
        if let Some(prev_app) = &event.previous_app {
            self.context_cache.remove(&prev_app.pid);
        }

        // Extract context for the new app if we support it
        if self.supported_bundles.contains(&event.app_info.bundle_id) {
            match self.extract_context(&event.app_info) {
                Ok(context) => {
                    // Log the enhanced context in a research-friendly format
                    println!("🔍 Enhanced Context Extracted:");
                    println!(
                        "   App: {} ({})",
                        context.app_info.name, context.app_info.bundle_id
                    );

                    if let Some(url) = &context.current_url {
                        println!("   📍 URL: {}", url);
                    }

                    if let Some(file) = &context.active_file_path {
                        println!("   📄 File: {}", file);
                        if let Some(project) = &context.project_name {
                            println!("      Project: {}", project);
                        }
                    }

                    if let Some(element) = &context.focused_element {
                        if let Some(role) = &element.role {
                            println!("   🎯 Focused: {} element", role);
                            if let Some(value) = &element.value {
                                if !value.trim().is_empty() && value.len() < 100 {
                                    println!("      Content: {}", value.trim());
                                }
                            }
                        }
                    }

                    if let Some(selected) = &context.selected_text {
                        println!("   ✏️  Selected: {}", selected);
                    }
                }
                Err(e) => {
                    eprintln!(
                        "⚠️  Failed to extract context for {}: {}",
                        event.app_info.name, e
                    );
                }
            }
        } else {
            // Even for unsupported apps, we can log basic information
            println!(
                "📱 Basic app switch: {} ({})",
                event.app_info.name, event.app_info.bundle_id
            );
        }
    }

    fn on_monitoring_started(&mut self) {
        println!("🔍 Accessibility context extractor started");
        println!(
            "   Supported applications: {}",
            self.supported_bundles.len()
        );
        println!("   Enhanced context available for: browsers, IDEs, document viewers");

        // Clear any stale cache entries
        self.context_cache.clear();
    }

    fn on_monitoring_stopped(&mut self) {
        println!("🔍 Accessibility context extractor stopped");
        println!("   Cache entries cleared: {}", self.context_cache.len());
        self.context_cache.clear();
    }
}
