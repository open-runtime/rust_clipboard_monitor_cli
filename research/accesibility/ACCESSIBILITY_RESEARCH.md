# macOS Accessibility Attributes Research

## Critical Finding: URL Extraction Issue
The focused element's VALUE contains the actual current URL, not the AXURL attribute!
Example: `focused_element.value = "en.wikipedia.org/wiki/Ohio"` while `url = "old perplexity url"`

## Comprehensive Accessibility Attributes by App Category

### Web Browsers (Chrome, Safari, Firefox, Edge, Arc, Brave)

#### Primary URL Sources (in order of reliability):
1. **AXValue** of focused text field with role "AXTextField" and description containing "Address"
2. **AXValue** of element with role "AXComboBox" (address bar)
3. **AXURL** attribute of window or web area
4. **AXDocument** attribute 
5. **AXDescription** of toolbar items
6. **AXTitle** of current tab
7. **AXValue** of selected tab
8. **AXLinkedUIElements** for tab connections

#### Browser-Specific Attributes:
- **Chrome/Chromium**: 
  - AXTextField with description "Address and search bar"
  - AXWebArea for page content
  - AXTabGroup for tab management
  
- **Safari**:
  - AXBrowserAddress
  - AXWebDocumentURL
  - AXWebAreaURL
  
- **Firefox**:
  - AXLocationBar
  - AXURLField

### IDEs and Code Editors

#### Cursor/VS Code:
- **AXDocument** - Full file path (file:// URL)
- **AXFilename** - Just the filename
- **AXPath** - Full path
- **AXDescription** - Often contains file info
- **AXTitleUIElement** - Tab title with filename
- **AXTabsGroup** - Open tabs list

#### JetBrains (IntelliJ, WebStorm, etc):
- **AXFilePath**
- **AXProjectPath** 
- **AXDocumentURI**
- **AXEditorTab**

#### Xcode:
- **AXDocumentPath**
- **AXSourceFile**
- **AXProjectName**
- **AXWorkspacePath**

### Terminal Applications

#### Terminal.app / iTerm2:
- **AXSelectedText** - Current selection
- **AXValue** - Terminal content
- **AXNumberOfCharacters** - Content length
- **AXSelectedTextRange** - Selection range
- **AXVisibleCharacterRange** - Visible portion
- **AXInsertionPointLineNumber** - Cursor position
- **AXTerminalSessionInfo** - Session metadata

### File Managers

#### Finder:
- **AXURL** / **AXDocument** - Current folder path
- **AXSelectedRows** - Selected items
- **AXSelectedChildren** - Selected files/folders
- **AXPath** - Item paths
- **AXFilename** - Item names
- **AXFileSize** - File sizes
- **AXDateModified** - Modification dates
- **AXFinderItem** - Item metadata

### Communication Apps

#### Slack:
- **AXConversationName**
- **AXMessageList**
- **AXChannelName**
- **AXWorkspaceName**

#### Discord:
- **AXChannelList**
- **AXServerName**
- **AXVoiceChannelName**

#### Messages:
- **AXConversationTitle**
- **AXMessageContent**
- **AXContactName**

### Productivity Apps

#### Notes:
- **AXNoteTitle**
- **AXNoteContent**
- **AXFolderName**
- **AXNoteModificationDate**

#### Calendar:
- **AXEventTitle**
- **AXEventTime**
- **AXEventLocation**
- **AXCalendarName**

#### Mail:
- **AXMailSubject**
- **AXMailSender**
- **AXMailContent**
- **AXMailboxName**

### Media Apps

#### Music/Spotify:
- **AXTrackName**
- **AXArtistName**
- **AXAlbumName**
- **AXPlaybackState**
- **AXCurrentTime**

#### Preview/PDF Viewers:
- **AXDocumentURI**
- **AXPageNumber**
- **AXNumberOfPages**
- **AXZoomLevel**

### System Attributes (Universal)

#### Window/Application Level:
- **AXFocusedWindow**
- **AXMainWindow**
- **AXWindows** (all windows)
- **AXMinimized**
- **AXFullScreen**

#### UI Navigation:
- **AXParent**
- **AXChildren**
- **AXTopLevelUIElement**
- **AXFocusedUIElement**
- **AXSelectedChildren**

#### Content:
- **AXValue**
- **AXTitle**
- **AXDescription**
- **AXRoleDescription**
- **AXHelp**
- **AXLabel**
- **AXPlaceholderValue**

#### State:
- **AXEnabled**
- **AXFocused**
- **AXSelected**
- **AXExpanded**
- **AXEdited**

#### Identification:
- **AXIdentifier**
- **AXIndex**
- **AXRole**
- **AXSubrole**

### Special Attributes for Deep Mining

#### Links and References:
- **AXLinkedUIElements**
- **AXServesAsTitleForUIElements**
- **AXTitleUIElement**

#### Scrolling and Position:
- **AXHorizontalScrollBar**
- **AXVerticalScrollBar**
- **AXScrollArea**
- **AXVisibleChildren**
- **AXPosition**
- **AXSize**

#### Text-specific:
- **AXNumberOfCharacters**
- **AXSelectedText**
- **AXSelectedTextRange**
- **AXSelectedTextRanges**
- **AXVisibleCharacterRange**
- **AXSharedTextUIElements**
- **AXSharedCharacterRange**

#### Tables/Lists:
- **AXRows**
- **AXVisibleRows**
- **AXSelectedRows**
- **AXColumns**
- **AXVisibleColumns**
- **AXSelectedColumns**
- **AXColumnTitles**
- **AXHeader**

## Implementation Strategy

1. **Always check focused element's AXValue first** for URLs in browsers
2. **Search multiple UI hierarchy levels** - don't stop at window level
3. **Use role-specific searches** - AXTextField with "Address" in description
4. **Cache nothing** - always re-extract on updates
5. **Combine multiple attributes** - some apps split info across attributes
6. **Check AXLinkedUIElements** - tabs often link to their content
7. **Monitor AXValueChanged** notifications for real-time updates

## Key Discovery Patterns

### For URLs:
```
1. Find AXTextField/AXComboBox with "address"/"search" in description
2. Get its AXValue (NOT AXURL)
3. Fallback to AXURL, AXDocument on window/web area
4. Parse from window title as last resort
```

### For File Paths:
```
1. Check AXDocument first (often has full path)
2. Then AXPath, AXFilename
3. Check window AXTitle for project context
4. Look in tab groups for open files list
```

### For Context:
```
1. Get AXFocusedUIElement
2. Walk up with AXParent to build hierarchy
3. Check AXDescription at each level
4. Look for AXLinkedUIElements for related content
```