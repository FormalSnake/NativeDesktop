// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import mermaid from 'astro-mermaid';

// https://astro.build/config
export default defineConfig({
	integrations: [
		// Must precede starlight so its rehype pass sees `mermaid` code blocks
		// first. autoTheme follows Starlight's light/dark toggle.
		mermaid({ autoTheme: true }),
		starlight({
			title: 'NativeDesktop',
			social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/FormalSnake/NativeDesktop' }],
			sidebar: [
				{
					label: 'Get Started',
					items: [
						{ label: 'Introduction', slug: 'get-started/introduction' },
						{ label: 'Quick Start', slug: 'get-started/quick-start' },
						{ label: 'Project Layout', slug: 'get-started/project-layout' },
						{ label: 'Monorepo & Code Sharing', slug: 'get-started/monorepo' },
					],
				},
				{
					label: 'Core Concepts',
					items: [
						{ label: 'Architecture', slug: 'core-concepts/architecture' },
						{ label: 'App Model', slug: 'core-concepts/app-model' },
						{ label: 'State & Hot Reload', slug: 'core-concepts/state-hot-reload' },
						{ label: 'Error Handling', slug: 'core-concepts/error-handling' },
						{ label: 'Styling & Design Language', slug: 'core-concepts/styling-design-language' },
						{ label: 'Automation-First', slug: 'core-concepts/automation-first' },
						{ label: 'Imperative Commands & Refs', slug: 'core-concepts/imperative-commands' },
						{ label: 'App Data & Storage', slug: 'core-concepts/app-data-storage' },
						{ label: 'RPC Client', slug: 'core-concepts/rpc-client' },
					],
				},
				{
					label: 'Components',
					items: [
						{ label: 'Overview', slug: 'components/overview' },
						{ label: 'Widget Reference', slug: 'components/widget-reference' },
						{ label: 'Terminal', slug: 'components/terminal' },
						{ label: 'WebView', slug: 'components/webview' },
						{ label: 'Form Controls', slug: 'components/form-controls' },
						{ label: 'Pickers', slug: 'components/pickers' },
						{ label: 'Menus & Popovers', slug: 'components/menus-and-popovers' },
						{ label: 'Command Palette', slug: 'components/command-palette' },
						{ label: 'Feedback', slug: 'components/feedback' },
						{ label: 'Data Views', slug: 'components/data-views' },
						{ label: 'Video', slug: 'components/video' },
						{ label: 'Dialogs', slug: 'components/dialogs' },
					],
				},
				{
					label: 'Native Platform',
					items: [
						{ label: 'Windows & Chrome', slug: 'native-platform/windows-chrome' },
						{ label: 'Split Views', slug: 'native-platform/split-views' },
						{ label: 'Menu Bar', slug: 'native-platform/menu-bar' },
						{ label: 'Multi-Window', slug: 'native-platform/multi-window' },
						{ label: 'Native Tabs', slug: 'native-platform/tabs' },
						{ label: 'System Capabilities', slug: 'native-platform/system-capabilities' },
						{ label: 'Native Modules', slug: 'native-platform/native-modules' },
						{ label: 'Icons', slug: 'native-platform/icons' },
						{ label: 'Platform Support', slug: 'native-platform/platform-support' },
					],
				},
				{
					label: 'Automation & Testing',
					items: [
						{ label: 'Automation Socket', slug: 'automation-testing/automation-socket' },
						{ label: 'Test Harness', slug: 'automation-testing/test-harness' },
						{ label: 'MCP Tools', slug: 'automation-testing/mcp-tools' },
					],
				},
				{
					label: 'Packaging',
					items: [{ label: 'Packaging', slug: 'packaging' }],
				},
			],
		}),
	],
});
