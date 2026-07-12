// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// https://astro.build/config
export default defineConfig({
	integrations: [
		starlight({
			title: 'NativeDesktop',
			social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/nativedesktop/nativedesktop' }],
			sidebar: [
				{
					label: 'Get Started',
					items: [
						{ label: 'Introduction', slug: 'get-started/introduction' },
						{ label: 'Quick Start', slug: 'get-started/quick-start' },
						{ label: 'Project Layout', slug: 'get-started/project-layout' },
					],
				},
				{
					label: 'Core Concepts',
					items: [
						{ label: 'App Model', slug: 'core-concepts/app-model' },
						{ label: 'State & Hot Reload', slug: 'core-concepts/state-hot-reload' },
						{ label: 'Styling & Design Language', slug: 'core-concepts/styling-design-language' },
						{ label: 'Automation-First', slug: 'core-concepts/automation-first' },
					],
				},
				{
					label: 'Components',
					items: [
						{ label: 'Overview', slug: 'components/overview' },
						{ label: 'Widget Reference', slug: 'components/widget-reference' },
					],
				},
				{
					label: 'Native Platform',
					items: [
						{ label: 'Windows & Chrome', slug: 'native-platform/windows-chrome' },
						{ label: 'Menu Bar', slug: 'native-platform/menu-bar' },
						{ label: 'Split Views', slug: 'native-platform/split-views' },
						{ label: 'Icons', slug: 'native-platform/icons' },
						{ label: 'Platform Support', slug: 'native-platform/platform-support' },
					],
				},
				{
					label: 'Automation & Testing',
					items: [
						{ label: 'Automation Socket', slug: 'automation-testing/automation-socket' },
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
