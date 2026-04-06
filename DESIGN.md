# Nebula Download Manager: Design Documentation

## 1. Design Vision
The Nebula DM redesign aims to transform a traditional, utility-heavy tool into a modern, high-tech command center. The aesthetic is "Sleek Professionalism" — combining the efficiency of a power user tool with the visual polish of a modern SaaS application.

## 2. Core Principles
*   **Information Density with Clarity:** Provide all necessary data (speed, progress, source) without overwhelming the user. Use clear visual hierarchy.
*   **Dynamic Feedback:** Use subtle animations and color-coded status indicators (e.g., orange for torrents, blue for direct downloads) to provide instant context.
*   **Unified Experience:** Seamlessly blend direct HTTP downloads and BitTorrent flows into a single, cohesive interface.

## 3. Visual Language

### Color Palette
*   **Background (Deep Space):** `#0F172A` - A dark, navy-tinted charcoal for the main workspace.
*   **Surface (Orbit):** `#1E293B` - Lighter slate for cards, sidebars, and elevated elements.
*   **Primary Action (Nebula Blue):** `#3B82F6` - Used for direct downloads and primary buttons.
*   **Secondary Action (Pulsar Orange):** `#F97316` - Used for torrent-related actions and statuses.
*   **Text (Starry White):** `#F8FAFC` (Primary) and `#94A3B8` (Secondary/Muted).

### Typography
*   **Font Family:** Inter or Roboto (Sans-serif)
*   **Headers:** Semi-bold, tight letter spacing for a technical look.
*   **Data Points:** Monospace fonts (e.g., JetBrains Mono) for speed numbers and file sizes to ensure alignment in tables.

## 4. Key Components

### Sidebar Navigation
*   **Active:** Real-time counter for currently running tasks.
*   **Queued/Completed:** Clear separation of historical and upcoming data.
*   **Torrents:** Dedicated section for seed/peer management.

### Quick Add Section
*   A prominent, simplified input area that auto-detects URL vs. Magnet links.
*   One-click "Direct" vs "Torrent" toggle to override automatic detection if needed.

### Download List (The Grid)
*   **Progress Bars:** Thin, high-contrast bars with glow effects.
*   **Action Icons:** Minimalist icons for Pause, Resume, Edit, and Delete, appearing on hover to reduce visual noise.

## 5. Interaction Design
*   **Drag & Drop:** Support for dragging files into the window to initiate uploads/torrents.
*   **System Tray Integration:** A "Ready" state in the tray for background operations, with a simplified context menu for quick controls.
