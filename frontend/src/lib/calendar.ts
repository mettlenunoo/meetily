import { invoke } from '@tauri-apps/api/core';

export interface CalendarSettings {
  enabled: boolean;
  reminders: boolean;
}

export interface CalendarStatus {
  settings: CalendarSettings;
  access: 'granted' | 'denied' | 'not_asked' | 'write_only' | 'unsupported';
}

export interface CalEvent {
  id: string;
  title: string;
  calendar: string | null;
  start: number;
  end: number;
  attendees: string[];
  allDay: boolean;
}

export interface MeetingEvent {
  eventTitle: string;
  calendarName: string | null;
  startsAt: string;
  endsAt: string;
  attendees: string | null;
}

export const calendarApi = {
  status: () => invoke<CalendarStatus>('api_calendar_status'),
  set: (settings: CalendarSettings) => invoke<CalendarStatus>('api_set_calendar', { settings }),
  upcoming: (hours = 12) => invoke<CalEvent[]>('api_upcoming_events', { hours }),
  forMeeting: (meetingId: string) => invoke<MeetingEvent | null>('api_meeting_event', { meetingId }),
};
