import {
  createIntegration,
  createTool,
  DapTool,
  DistriPlugin,
  ExecutionContext,
} from "https://distri.dev/base.ts";

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
  access_token?: string;
}

interface CreateEventParams {
  calendarId?: string;
  event: CalendarEvent;
  sendNotifications?: boolean;
  access_token?: string;
}

interface UpdateEventParams {
  calendarId?: string;
  eventId: string;
  event: Partial<CalendarEvent>;
  sendNotifications?: boolean;
  access_token?: string;
}

function resolveAccessToken(params: Record<string, unknown>, context?: ExecutionContext): string {
  // 1. Check params.access_token
  if (params.access_token) {
    return params.access_token as string;
  }

  // 2. Check secrets
  const secrets = context?.secrets || {};
  const candidates = [
    "google_access_token",
    "google_calendar_access_token",
    "google",
    "google_calendar",
  ];

  for (const key of candidates) {
    if (secrets[key]) {
      return secrets[key];
    }
  }

  // 3. Check auth_session (OAuth flow)
  if (context?.auth_session?.access_token) {
    return context.auth_session.access_token as string;
  }

  throw new Error("Google Calendar requires authentication. Provide access_token parameter, configure google_access_token secret, or use OAuth flow.");
}

class GoogleCalendarIntegration {
  name = "google_calendar";
  version = "1.0.0";
  description = "Google Calendar integration with OAuth2 authentication";

  private baseUrl = "https://www.googleapis.com/calendar/v3";
  private accessToken: string;

  constructor(accessToken: string) {
    this.accessToken = accessToken;
  }

  private async makeRequest(endpoint: string, options: RequestInit = {}) {
    const headers = {
      Authorization: `Bearer ${this.accessToken}`,
      "Content-Type": "application/json",
      ...options.headers,
    } as Record<string, string>;

    const response = await fetch(`${this.baseUrl}${endpoint}`, {
      ...options,
      headers,
    });

    if (!response.ok) {
      const error = await response.text();
      if (response.status === 401) {
        throw new Error("Google Calendar authentication failed. Refresh the OAuth token.");
      }

      throw new Error(`Google Calendar API error (${response.status}): ${error}`);
    }

    if (response.status === 204) {
      return {};
    }

    return await response.json();
  }

  async listEvents(params: CalendarListParams = {}) {
    const calendarId = params.calendarId || "primary";
    const queryParams = new URLSearchParams();

    if (params.maxResults) queryParams.append("maxResults", params.maxResults.toString());
    if (params.timeMin) queryParams.append("timeMin", params.timeMin);
    if (params.timeMax) queryParams.append("timeMax", params.timeMax);
    if (params.singleEvents !== undefined) queryParams.append("singleEvents", String(params.singleEvents));
    if (params.orderBy) queryParams.append("orderBy", params.orderBy);

    const endpoint = `/calendars/${encodeURIComponent(calendarId)}/events?${queryParams.toString()}`;
    return await this.makeRequest(endpoint);
  }

  async createEvent(params: CreateEventParams) {
    const calendarId = params.calendarId || "primary";
    const queryParams = new URLSearchParams();

    if (params.sendNotifications !== undefined) {
      queryParams.append("sendNotifications", String(params.sendNotifications));
    }

    const endpoint = `/calendars/${encodeURIComponent(calendarId)}/events?${queryParams.toString()}`;
    return await this.makeRequest(endpoint, {
      method: "POST",
      body: JSON.stringify(params.event),
    });
  }

  async updateEvent(params: UpdateEventParams) {
    const calendarId = params.calendarId || "primary";
    const queryParams = new URLSearchParams();

    if (params.sendNotifications !== undefined) {
      queryParams.append("sendNotifications", String(params.sendNotifications));
    }

    const endpoint = `/calendars/${encodeURIComponent(calendarId)}/events/${encodeURIComponent(params.eventId)}?${queryParams.toString()}`;
    return await this.makeRequest(endpoint, {
      method: "PUT",
      body: JSON.stringify(params.event),
    });
  }

  async deleteEvent(calendarId: string, eventId: string, sendNotifications = false) {
    const queryParams = new URLSearchParams();

    if (sendNotifications) {
      queryParams.append("sendNotifications", "true");
    }

    const endpoint = `/calendars/${encodeURIComponent(calendarId)}/events/${encodeURIComponent(eventId)}?${queryParams.toString()}`;
    return await this.makeRequest(endpoint, {
      method: "DELETE",
    });
  }

  async getEvent(calendarId: string, eventId: string) {
    const endpoint = `/calendars/${encodeURIComponent(calendarId)}/events/${encodeURIComponent(eventId)}`;
    return await this.makeRequest(endpoint);
  }

  async listCalendars() {
    return await this.makeRequest("/users/me/calendarList");
  }

  async getFreeBusy(params: {
    timeMin: string;
    timeMax: string;
    items: Array<{ id: string }>;
    timeZone?: string;
  }) {
    return await this.makeRequest("/freeBusy", {
      method: "POST",
      body: JSON.stringify(params),
    });
  }

  async testConnection() {
    const response = await this.makeRequest("/users/me/calendarList/primary");
    return {
      success: true,
      calendar: response.summary,
      id: response.id,
      response,
    };
  }
}

function getCalendarTools(): DapTool[] {
  return [
    createTool({
      name: "list_events",
      description: "List upcoming events from Google Calendar.",
      auth: {
        type: "oauth2",
        provider: "google",
        scopes: ["https://www.googleapis.com/auth/calendar.readonly"],
      },
      parameters: {
        type: "object",
        properties: {
          calendarId: { type: "string", description: "Calendar ID, default primary." },
          maxResults: { type: "number", description: "Maximum events to return." },
          timeMin: { type: "string", description: "Start time (ISO 8601)." },
          timeMax: { type: "string", description: "End time (ISO 8601)." },
          days: { type: "number", description: "Shortcut to fetch next N days." },
          access_token: { type: "string", description: "Google OAuth access token (optional if configured in secrets)." },
        },
      },
      execute: async (params, context) => {
        const accessToken = resolveAccessToken(params, context);
        const calendar = new GoogleCalendarIntegration(accessToken);

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
          orderBy: "startTime",
        });
      },
    }),
    createTool({
      name: "create_event",
      description: "Create a Google Calendar event.",
      parameters: {
        type: "object",
        properties: {
          summary: { type: "string", description: "Event summary." },
          description: { type: "string", description: "Event description." },
          startDateTime: { type: "string", description: "Start time (ISO 8601)." },
          endDateTime: { type: "string", description: "End time (ISO 8601)." },
          timeZone: { type: "string", description: "Time zone." },
          location: { type: "string", description: "Event location." },
          attendees: {
            type: "array",
            items: { type: "string" },
            description: "Attendee email addresses.",
          },
          calendarId: { type: "string", description: "Target calendar ID." },
          sendNotifications: { type: "boolean", description: "Notify attendees." },
          access_token: { type: "string", description: "Google OAuth access token (optional if configured in secrets)." },
        },
        required: ["summary", "startDateTime", "endDateTime"],
      },
      execute: async (params, context) => {
        const accessToken = resolveAccessToken(params, context);
        const calendar = new GoogleCalendarIntegration(accessToken);
        const event: CalendarEvent = {
          summary: params.summary,
          description: params.description,
          start: {
            dateTime: params.startDateTime,
            timeZone: params.timeZone || "UTC",
          },
          end: {
            dateTime: params.endDateTime,
            timeZone: params.timeZone || "UTC",
          },
          location: params.location,
          attendees: params.attendees?.map((email: string) => ({ email })),
        };

        return await calendar.createEvent({
          calendarId: params.calendarId,
          event,
          sendNotifications: params.sendNotifications,
        });
      },
    }),
    createTool({
      name: "update_event",
      description: "Update an existing Google Calendar event.",
      parameters: {
        type: "object",
        properties: {
          eventId: { type: "string", description: "Event ID to update." },
          calendarId: { type: "string", description: "Calendar ID." },
          summary: { type: "string", description: "New summary." },
          description: { type: "string", description: "New description." },
          startDateTime: { type: "string", description: "New start time." },
          endDateTime: { type: "string", description: "New end time." },
          timeZone: { type: "string", description: "Time zone." },
          sendNotifications: { type: "boolean", description: "Notify attendees." },
          access_token: { type: "string", description: "Google OAuth access token (optional if configured in secrets)." },
        },
        required: ["eventId"],
      },
      execute: async (params, context) => {
        const accessToken = resolveAccessToken(params, context);
        const calendar = new GoogleCalendarIntegration(accessToken);
        const event: Partial<CalendarEvent> = {};

        if (params.summary) event.summary = params.summary;
        if (params.description) event.description = params.description;
        if (params.startDateTime) {
          event.start = {
            dateTime: params.startDateTime,
            timeZone: params.timeZone || "UTC",
          };
        }
        if (params.endDateTime) {
          event.end = {
            dateTime: params.endDateTime,
            timeZone: params.timeZone || "UTC",
          };
        }

        return await calendar.updateEvent({
          calendarId: params.calendarId,
          eventId: params.eventId,
          event,
          sendNotifications: params.sendNotifications,
        });
      },
    }),
    createTool({
      name: "delete_event",
      description: "Delete a Google Calendar event.",
      parameters: {
        type: "object",
        properties: {
          eventId: { type: "string", description: "Event ID." },
          calendarId: { type: "string", description: "Calendar ID." },
          sendNotifications: { type: "boolean", description: "Notify attendees." },
          access_token: { type: "string", description: "Google OAuth access token (optional if configured in secrets)." },
        },
        required: ["eventId"],
      },
      execute: async (params, context) => {
        const accessToken = resolveAccessToken(params, context);
        const calendar = new GoogleCalendarIntegration(accessToken);
        const calendarId = params.calendarId || "primary";
        await calendar.deleteEvent(calendarId, params.eventId, params.sendNotifications ?? false);
        return { success: true };
      },
    }),
    createTool({
      name: "get_event",
      description: "Fetch a single Google Calendar event.",
      parameters: {
        type: "object",
        properties: {
          eventId: { type: "string", description: "Event ID." },
          calendarId: { type: "string", description: "Calendar ID." },
          access_token: { type: "string", description: "Google OAuth access token (optional if configured in secrets)." },
        },
        required: ["eventId"],
      },
      execute: async (params, context) => {
        const accessToken = resolveAccessToken(params, context);
        const calendar = new GoogleCalendarIntegration(accessToken);
        const calendarId = params.calendarId || "primary";
        return await calendar.getEvent(calendarId, params.eventId);
      },
    }),
    createTool({
      name: "list_calendars",
      description: "List calendars available for the authenticated user.",
      parameters: {
        type: "object",
        properties: {
          access_token: { type: "string", description: "Google OAuth access token (optional if configured in secrets)." },
        },
      },
      execute: async (params, context) => {
        const accessToken = resolveAccessToken(params, context);
        const calendar = new GoogleCalendarIntegration(accessToken);
        return await calendar.listCalendars();
      },
    }),
    createTool({
      name: "free_busy",
      description: "Query free/busy information for calendars.",
      parameters: {
        type: "object",
        properties: {
          timeMin: { type: "string", description: "Start time (ISO 8601)." },
          timeMax: { type: "string", description: "End time (ISO 8601)." },
          items: {
            type: "array",
            description: "Calendars to query.",
            items: {
              type: "object",
              properties: {
                id: { type: "string" },
              },
              required: ["id"],
            },
          },
          timeZone: { type: "string", description: "Time zone override." },
          access_token: { type: "string", description: "Google OAuth access token (optional if configured in secrets)." },
        },
        required: ["timeMin", "timeMax", "items"],
      },
      execute: async (params, context) => {
        const accessToken = resolveAccessToken(params, context);
        const calendar = new GoogleCalendarIntegration(accessToken);
        return await calendar.getFreeBusy(params);
      },
    }),
    createTool({
      name: "test_connection",
      description: "Verify the Google Calendar token is valid.",
      parameters: {
        type: "object",
        properties: {
          access_token: { type: "string", description: "Google OAuth access token (optional if configured in secrets)." },
        },
      },
      execute: async (params, context) => {
        const accessToken = resolveAccessToken(params, context);
        const calendar = new GoogleCalendarIntegration(accessToken);
        return await calendar.testConnection();
      },
    }),
  ];
}

const googleCalendarPlugin: DistriPlugin = {
  integrations: [
    createIntegration({
      name: "google_calendar",
      description: "Google Calendar integration for scheduling workflows.",
      version: "1.0.0",
      tools: getCalendarTools(),
      auth: {
        type: "oauth2",
        provider: "google",
        authorizationUrl: "https://accounts.google.com/o/oauth2/v2/auth",
        tokenUrl: "https://oauth2.googleapis.com/token",
        refreshUrl: "https://oauth2.googleapis.com/token",
        scopes: [
          "https://www.googleapis.com/auth/calendar",
          "https://www.googleapis.com/auth/calendar.readonly",
          "https://www.googleapis.com/auth/calendar.events",
        ],
      },
      metadata: {
        category: "productivity",
        documentation: "https://developers.google.com/calendar/api",
        redirect_uri: "/auth/google/callback",
      },
    }),
  ],
  workflows: [],
};

export default googleCalendarPlugin;
