/**
 * Google Calendar Integration for Distri
 * Requires Google OAuth2 authentication
 */

import { createTool, DapTool, Context } from "https://distri.dev/base.ts";

interface CalendarEvent {
    id?: string;
    summary: string;
    description?: string;
    start: {
        dateTime?: string;
        date?: string;
        timeZone?: string;
    };
    end: {
        dateTime?: string;
        date?: string;
        timeZone?: string;
    };
    attendees?: Array<{
        email: string;
        displayName?: string;
        responseStatus?: string;
    }>;
    location?: string;
    reminders?: {
        useDefault?: boolean;
        overrides?: Array<{
            method: string;
            minutes: number;
        }>;
    };
}

interface CalendarListParams {
    calendarId?: string;
    maxResults?: number;
    timeMin?: string;
    timeMax?: string;
    singleEvents?: boolean;
    orderBy?: string;
}

interface CreateEventParams {
    calendarId?: string;
    event: CalendarEvent;
    sendNotifications?: boolean;
}

interface UpdateEventParams {
    calendarId?: string;
    eventId: string;
    event: Partial<CalendarEvent>;
    sendNotifications?: boolean;
}

interface CalendarConfig {
    accessToken: string;
}

// Get config from context
function getCalendarConfig(context: any): CalendarConfig {    
    const accessToken = context?.auth_session?.access_token;
    
    if (!accessToken) {
        throw new Error('🔐 Tool \'listEvents\' requires authentication\n💡 Run: /auth login google calendar.readonly');
    }
    
    return { accessToken };
}

export class GoogleCalendarIntegration {
    name = "google_calendar";
    version = "1.0.0";
    description = "Google Calendar integration with OAuth2 authentication";

    private baseUrl = "https://www.googleapis.com/calendar/v3";
    private accessToken: string;

    constructor(context?: Context) {
        const config = getCalendarConfig(context);
        this.accessToken = config.accessToken;
    }

    /**
     * Set the access token for API calls (for manual override)
     */
    setAccessToken(token: string) {
        this.accessToken = token;
    }

    /**
     * Get the access token, throwing error if not available
     */
    private getAccessToken(): string {
        return this.accessToken;
    }

    /**
     * Make an authenticated request to Google Calendar API
     */
    private async makeRequest(endpoint: string, options: RequestInit = {}): Promise<any> {
        const token = this.getAccessToken();

        const url = `${this.baseUrl}${endpoint}`;
        const headers = {
            'Authorization': `Bearer ${token}`,
            'Content-Type': 'application/json',
            ...options.headers,
        };

        // Calendar API request will be logged by the runtime if needed

        try {
            const response = await fetch(url, {
                ...options,
                headers,
            });

            if (!response.ok) {
                const error = await response.text();

                if (response.status === 401) {
                    throw new Error(`🔐 Authentication failed. Your Google session may have expired. Please run: /auth login google calendar`);
                }

                throw new Error(`Google Calendar API error (${response.status}): ${error}`);
            }

            return await response.json();
        } catch (error) {
            throw error;
        }
    }

    /**
     * List upcoming events from the primary calendar
     */
    async listEvents(params: CalendarListParams = {}): Promise<any> {
        const calendarId = params.calendarId || 'primary';

        const queryParams = new URLSearchParams();
        if (params.maxResults) queryParams.append('maxResults', params.maxResults.toString());
        if (params.timeMin) queryParams.append('timeMin', params.timeMin);
        if (params.timeMax) queryParams.append('timeMax', params.timeMax);
        if (params.singleEvents !== undefined) queryParams.append('singleEvents', params.singleEvents.toString());
        if (params.orderBy) queryParams.append('orderBy', params.orderBy);

        const endpoint = `/calendars/${encodeURIComponent(calendarId)}/events?${queryParams.toString()}`;
        return await this.makeRequest(endpoint);
    }

    /**
     * Create a new calendar event
     */
    async createEvent(params: CreateEventParams): Promise<any> {
        const calendarId = params.calendarId || 'primary';

        const queryParams = new URLSearchParams();
        if (params.sendNotifications !== undefined) {
            queryParams.append('sendNotifications', params.sendNotifications.toString());
        }

        const endpoint = `/calendars/${encodeURIComponent(calendarId)}/events?${queryParams.toString()}`;

        return await this.makeRequest(endpoint, {
            method: 'POST',
            body: JSON.stringify(params.event),
        });
    }

    /**
     * Update an existing calendar event
     */
    async updateEvent(params: UpdateEventParams): Promise<any> {
        const calendarId = params.calendarId || 'primary';

        const queryParams = new URLSearchParams();
        if (params.sendNotifications !== undefined) {
            queryParams.append('sendNotifications', params.sendNotifications.toString());
        }

        const endpoint = `/calendars/${encodeURIComponent(calendarId)}/events/${encodeURIComponent(params.eventId)}?${queryParams.toString()}`;

        return await this.makeRequest(endpoint, {
            method: 'PUT',
            body: JSON.stringify(params.event),
        });
    }

    /**
     * Delete a calendar event
     */
    async deleteEvent(calendarId: string = 'primary', eventId: string, sendNotifications: boolean = false): Promise<any> {
        const queryParams = new URLSearchParams();
        if (sendNotifications) {
            queryParams.append('sendNotifications', 'true');
        }

        const endpoint = `/calendars/${encodeURIComponent(calendarId)}/events/${encodeURIComponent(eventId)}?${queryParams.toString()}`;

        return await this.makeRequest(endpoint, {
            method: 'DELETE',
        });
    }

    /**
     * Get details of a specific event
     */
    async getEvent(calendarId: string = 'primary', eventId: string): Promise<any> {
        const endpoint = `/calendars/${encodeURIComponent(calendarId)}/events/${encodeURIComponent(eventId)}`;
        return await this.makeRequest(endpoint);
    }

    /**
     * List all calendars for the authenticated user
     */
    async listCalendars(): Promise<any> {
        const endpoint = '/users/me/calendarList';
        return await this.makeRequest(endpoint);
    }

    /**
     * Get free/busy information for calendars
     */
    async getFreeBusy(params: {
        timeMin: string;
        timeMax: string;
        items: Array<{ id: string }>;
        timeZone?: string;
    }): Promise<any> {
        const endpoint = '/freeBusy';
        return await this.makeRequest(endpoint, {
            method: 'POST',
            body: JSON.stringify(params),
        });
    }

    /**
     * Test the connection by getting user's primary calendar
     */
    async testConnection(): Promise<any> {
        try {
            const response = await this.makeRequest('/users/me/calendarList/primary');
            return {
                success: true,
                message: "Google Calendar connection test successful",
                calendar: response.summary,
                id: response.id,
                response
            };
        } catch (error) {
            throw error;
        }
    }
}

/**
 * Export tools for the plugin system
 * These tools require Google OAuth2 authentication with calendar scope
 */
export function getTools(): DapTool[] {
    return [
        createTool({
            name: "listEvents",
            description: "List upcoming calendar events from Google Calendar",
            requiresAuth: {
                provider: "google",
                scopes: ["https://www.googleapis.com/auth/calendar.readonly"]
            },
            parameters: {
                type: "object",
                properties: {
                    calendarId: {
                        type: "string",
                        description: "Calendar ID (defaults to 'primary')",
                        default: "primary"
                    },
                    maxResults: {
                        type: "number",
                        description: "Maximum number of events to return (default: 10)",
                        default: 10
                    },
                    timeMin: {
                        type: "string",
                        description: "Start time (ISO 8601 format)"
                    },
                    timeMax: {
                        type: "string",
                        description: "End time (ISO 8601 format)"
                    },
                    days: {
                        type: "number",
                        description: "Get events for the next N days (alternative to timeMin/timeMax)"
                    }
                },
                required: []
            },
            execute: async (params: {
                calendarId?: string;
                maxResults?: number;
                timeMin?: string;
                timeMax?: string;
                days?: number;
            }, context?: any) => {
                const calendar = new GoogleCalendarIntegration(context);

                // Helper: if days is provided, calculate timeMin and timeMax
                if (params.days && !params.timeMin && !params.timeMax) {
                    const now = new Date();
                    const future = new Date();
                    future.setDate(now.getDate() + params.days);

                    params.timeMin = now.toISOString();
                    params.timeMax = future.toISOString();
                }

                return await calendar.listEvents({
                    calendarId: params.calendarId,
                    maxResults: params.maxResults || 10,
                    timeMin: params.timeMin,
                    timeMax: params.timeMax,
                    singleEvents: true,
                    orderBy: 'startTime'
                });
            }
        }),

        createTool({
            name: "createEvent",
            description: "Create a new calendar event in Google Calendar",
            parameters: {
                type: "object",
                properties: {
                    summary: { type: "string", description: "Event title/summary" },
                    description: { type: "string", description: "Event description" },
                    startDateTime: { type: "string", description: "Event start time (ISO 8601 format)" },
                    endDateTime: { type: "string", description: "Event end time (ISO 8601 format)" },
                    timeZone: { type: "string", description: "Time zone (default: UTC)" },
                    location: { type: "string", description: "Event location" },
                    attendees: {
                        type: "array",
                        items: { type: "string" },
                        description: "List of attendee email addresses"
                    },
                    calendarId: { type: "string", description: "Calendar ID (default: primary)" },
                    sendNotifications: { type: "boolean", description: "Send email notifications to attendees" }
                },
                required: ["summary", "startDateTime", "endDateTime"]
            },
            execute: async (params: {
                summary: string;
                description?: string;
                startDateTime: string;
                endDateTime: string;
                timeZone?: string;
                location?: string;
                attendees?: string[];
                calendarId?: string;
                sendNotifications?: boolean;
            }, context?: any) => {
                const calendar = new GoogleCalendarIntegration(context);

                const event: CalendarEvent = {
                    summary: params.summary,
                    description: params.description,
                    start: {
                        dateTime: params.startDateTime,
                        timeZone: params.timeZone || 'UTC'
                    },
                    end: {
                        dateTime: params.endDateTime,
                        timeZone: params.timeZone || 'UTC'
                    },
                    location: params.location,
                    attendees: params.attendees?.map(email => ({ email }))
                };

                return await calendar.createEvent({
                    calendarId: params.calendarId,
                    event,
                    sendNotifications: params.sendNotifications
                });
            }
        }),

        createTool({
            name: "updateEvent",
            description: "Update an existing calendar event in Google Calendar",
            parameters: {
                type: "object",
                properties: {
                    eventId: { type: "string", description: "Event ID to update" },
                    summary: { type: "string", description: "Event title/summary" },
                    description: { type: "string", description: "Event description" },
                    startDateTime: { type: "string", description: "Event start time (ISO 8601 format)" },
                    endDateTime: { type: "string", description: "Event end time (ISO 8601 format)" },
                    timeZone: { type: "string", description: "Time zone" },
                    location: { type: "string", description: "Event location" },
                    attendees: {
                        type: "array",
                        items: { type: "string" },
                        description: "List of attendee email addresses"
                    },
                    calendarId: { type: "string", description: "Calendar ID (default: primary)" },
                    sendNotifications: { type: "boolean", description: "Send email notifications to attendees" }
                },
                required: ["eventId"]
            },
            execute: async (params: {
                eventId: string;
                summary?: string;
                description?: string;
                startDateTime?: string;
                endDateTime?: string;
                timeZone?: string;
                location?: string;
                attendees?: string[];
                calendarId?: string;
                sendNotifications?: boolean;
            }, context?: any) => {
                const calendar = new GoogleCalendarIntegration(context);

                const eventUpdates: Partial<CalendarEvent> = {};
                if (params.summary) eventUpdates.summary = params.summary;
                if (params.description) eventUpdates.description = params.description;
                if (params.startDateTime) {
                    eventUpdates.start = {
                        dateTime: params.startDateTime,
                        timeZone: params.timeZone || 'UTC'
                    };
                }
                if (params.endDateTime) {
                    eventUpdates.end = {
                        dateTime: params.endDateTime,
                        timeZone: params.timeZone || 'UTC'
                    };
                }
                if (params.location) eventUpdates.location = params.location;
                if (params.attendees) eventUpdates.attendees = params.attendees.map(email => ({ email }));

                return await calendar.updateEvent({
                    calendarId: params.calendarId,
                    eventId: params.eventId,
                    event: eventUpdates,
                    sendNotifications: params.sendNotifications
                });
            }
        }),

        createTool({
            name: "deleteEvent",
            description: "Delete a calendar event from Google Calendar",
            parameters: {
                type: "object",
                properties: {
                    eventId: { type: "string", description: "Event ID to delete" },
                    calendarId: { type: "string", description: "Calendar ID (default: primary)" },
                    sendNotifications: { type: "boolean", description: "Send cancellation notifications to attendees" }
                },
                required: ["eventId"]
            },
            execute: async (params: {
                eventId: string;
                calendarId?: string;
                sendNotifications?: boolean;
            }, context?: any) => {
                const calendar = new GoogleCalendarIntegration(context);

                const result = await calendar.deleteEvent(
                    params.calendarId || 'primary',
                    params.eventId,
                    params.sendNotifications || false
                );

                return {
                    success: true,
                    message: `Event ${params.eventId} deleted successfully`,
                    result
                };
            }
        }),

        createTool({
            name: "getEvent",
            description: "Get details of a specific calendar event",
            parameters: {
                type: "object",
                properties: {
                    eventId: { type: "string", description: "Event ID to retrieve" },
                    calendarId: { type: "string", description: "Calendar ID (default: primary)" }
                },
                required: ["eventId"]
            },
            execute: async (params: {
                eventId: string;
                calendarId?: string;
            }, context?: any) => {
                const calendar = new GoogleCalendarIntegration(context);
                return await calendar.getEvent(
                    params.calendarId || 'primary',
                    params.eventId
                );
            }
        }),

        createTool({
            name: "listCalendars",
            description: "List all calendars accessible to the authenticated user",
            parameters: {
                type: "object",
                properties: {},
                required: []
            },
            execute: async (params: {}, context?: any) => {
                const calendar = new GoogleCalendarIntegration(context);
                return await calendar.listCalendars();
            }
        }),

        createTool({
            name: "getFreeBusy",
            description: "Get free/busy information for calendars",
            parameters: {
                type: "object",
                properties: {
                    timeMin: { type: "string", description: "Start time for free/busy query (ISO 8601)" },
                    timeMax: { type: "string", description: "End time for free/busy query (ISO 8601)" },
                    calendarIds: {
                        type: "array",
                        items: { type: "string" },
                        description: "Calendar IDs to check (default: primary)"
                    },
                    timeZone: { type: "string", description: "Time zone for the query" }
                },
                required: ["timeMin", "timeMax"]
            },
            execute: async (params: {
                timeMin: string;
                timeMax: string;
                calendarIds?: string[];
                timeZone?: string;
            }, context?: any) => {
                const calendar = new GoogleCalendarIntegration(context);

                const items = (params.calendarIds || ['primary']).map(id => ({ id }));

                return await calendar.getFreeBusy({
                    timeMin: params.timeMin,
                    timeMax: params.timeMax,
                    items,
                    timeZone: params.timeZone
                });
            }
        }),

        createTool({
            name: "testConnection",
            description: "Test the Google Calendar API connection",
            parameters: {
                type: "object",
                properties: {},
                required: []
            },
            execute: async (params: {}, context?: any) => {
                const calendar = new GoogleCalendarIntegration(context);

                return await calendar.testConnection();
            }
        })
    ];
}

export default {
    getTools
}