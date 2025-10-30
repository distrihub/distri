import { DistriPlugin, createIntegration } from 'https://distri.dev/base.ts';
import slack_poet from './workflows/slack_poet.ts';
import { getTools as getSlackTools } from './tools/slack.ts';
import { getBuildTools } from './tools/build.ts';
import { getTools as getCalendarTools } from './tools/calendar.ts';

// Slack integration
const slackIntegration = createIntegration({
    name: 'slack',
    description: 'Simple Slack messaging integration',
    version: '1.0.0',
    tools: getSlackTools(),
    notifications: ['message_sent'],
    metadata: {
        documentation: 'https://api.slack.com/',
        category: 'messaging'
    }
});

// Google Calendar integration with OAuth2 authentication
const calendarIntegration = createIntegration({
    name: 'calendar',
    description: 'Google Calendar integration',
    version: '1.0.0',
    tools: getCalendarTools(),
    authProvider: {
        type: 'oauth',
        provider: 'google',
        authorization_url: 'https://accounts.google.com/o/oauth2/v2/auth',
        token_url: 'https://oauth2.googleapis.com/token',
        refresh_url: 'https://oauth2.googleapis.com/token',
        scopes: ['https://www.googleapis.com/auth/calendar', 'https://www.googleapis.com/auth/calendar.readonly', 'https://www.googleapis.com/auth/calendar.events'],
        redirect_uri: '/auth/google/callback'
    },
    notifications: ['event_created', 'event_updated', 'event_deleted'],
    metadata: {
        documentation: 'https://developers.google.com/calendar/api',
        category: 'productivity'
    }
});

// Build/Publishing integration (no auth needed)
const buildIntegration = createIntegration({
    name: 'build',
    description: 'Plugin build and publishing tools',
    version: '1.0.0',
    tools: getBuildTools(),
    notifications: ['build_complete', 'publish_complete'],
    metadata: {
        category: 'development'
    }
});

const plugin: DistriPlugin = {
    integrations: [slackIntegration, calendarIntegration, buildIntegration],
    workflows: [slack_poet],

};
export default plugin;