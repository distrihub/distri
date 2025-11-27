import { DistriFnTool } from "@distri/core";
import { GoogleMapsManagerRef } from "./components/GoogleMapsManager";

export const getTools = (mapManagerRef: GoogleMapsManagerRef): DistriFnTool[] => {

  return [
    {
      name: 'set_map_center',
      description: 'Set the center location of the Google Maps view',
      type: 'function',
      parameters: {
        type: 'object',
        properties: {
          latitude: { type: 'number', description: 'Latitude coordinate for the map center' },
          longitude: { type: 'number', description: 'Longitude coordinate for the map center' },
          zoom: { type: 'number', description: 'Zoom level (1-20)', minimum: 1, maximum: 20, default: 13 }
        },
        required: ['latitude', 'longitude']
      },
      handler: async (input: object) => {
        console.log('set_map_center', input);
        const { latitude, longitude, zoom } = input as { latitude: number; longitude: number; zoom: number };
        if (!latitude || !longitude) {
          return "Invalid input";
        }
        await mapManagerRef.setMapCenter({ latitude, longitude, zoom });
        return "Map center set to " + latitude + ", " + longitude;
      }
    } as DistriFnTool,

    {
      name: 'add_marker',
      description: 'Add a marker to the Google Maps at a specific location',
      type: 'function',
      parameters: {
        type: 'object',
        properties: {
          latitude: { type: 'number', description: 'Latitude coordinate for the marker' },
          longitude: { type: 'number', description: 'Longitude coordinate for the marker' },
          title: { type: 'string', description: 'Title/label for the marker' },
          description: { type: 'string', description: 'Optional description for the marker' }
        },
        required: ['latitude', 'longitude', 'title']
      },
      handler: async (input: object) => {
        const { latitude, longitude, title, description } = input as { latitude: number; longitude: number; title: string; description: string };
        if (!latitude || !longitude || !title) {
          return "Invalid input";
        }
        await mapManagerRef.addMarker({ latitude, longitude, title, description });
        return "Marker added";
      }
    } as DistriFnTool,

    {
      name: 'get_directions',
      description: 'Get directions between two locations on Google Maps',
      type: 'function',
      parameters: {
        type: 'object',
        properties: {
          origin: { type: 'string', description: 'Starting location (address or place name)' },
          destination: { type: 'string', description: 'Destination location (address or place name)' },
          travel_mode: {
            type: 'string',
            enum: ['DRIVING', 'WALKING', 'BICYCLING', 'TRANSIT'],
            default: 'DRIVING',
            description: 'Mode of transportation'
          }
        },
        required: ['origin', 'destination']
      },
      handler: async (input: object) => {
        const { origin, destination, travel_mode } = input as { origin: string; destination: string; travel_mode: string };
        if (!origin || !destination) {
          return "Invalid input";
        }
        const result = await mapManagerRef.getDirections({ origin, destination, travel_mode });
        return result;
      }
    } as DistriFnTool,

    {
      name: 'search_places',
      description: 'Search for places near a location',
      type: 'function',
      parameters: {
        type: 'object',
        properties: {
          query: { type: 'string', description: 'Search query (e.g., "restaurants", "gas stations")' },
          latitude: { type: 'number', description: 'Latitude for the search center' },
          longitude: { type: 'number', description: 'Longitude for the search center' },
          radius: { type: 'number', description: 'Search radius in meters', default: 5000 }
        },
        required: ['query', 'latitude', 'longitude']
      },
      handler: async (input: object) => {
        const { query, latitude, longitude, radius } = input as { query: string; latitude: number; longitude: number; radius: number };
        if (!query || !latitude || !longitude) {
          return "Invalid input";
        }
        const result = await mapManagerRef.searchPlaces({ query, latitude, longitude, radius });
        return JSON.stringify(result);
      }
    } as DistriFnTool,

    {
      name: 'geocode_address',
      description: 'Convert an address or place name to latitude and longitude coordinates',
      type: 'function',
      parameters: {
        type: 'object',
        properties: {
          address: { type: 'string', description: 'Address or place name to geocode (e.g., "1600 Amphitheatre Parkway, Mountain View, CA" or "Eiffel Tower, Paris")' }
        },
        required: ['address']
      },
      handler: async (input: object) => {
        const { address } = input as { address: string };
        if (!address) {
          return "Invalid input: address is required";
        }
        const result = await mapManagerRef.geocodeAddress({ address });
        if (result.success && result.coordinates) {
          return JSON.stringify({
            success: true,
            message: result.message,
            latitude: result.coordinates.latitude,
            longitude: result.coordinates.longitude,
            formatted_address: result.coordinates.formatted_address
          });
        }
        return JSON.stringify(result);
      }
    } as DistriFnTool,

    {
      name: 'get_place_details',
      description: 'Get detailed information about a place using its place_id (obtained from search_places results)',
      type: 'function',
      parameters: {
        type: 'object',
        properties: {
          place_id: { type: 'string', description: 'Place ID from Google Places API (obtained from search_places results)' }
        },
        required: ['place_id']
      },
      handler: async (input: object) => {
        const { place_id } = input as { place_id: string };
        if (!place_id) {
          return "Invalid input: place_id is required";
        }
        const result = await mapManagerRef.getPlaceDetails({ place_id });
        return JSON.stringify(result);
      }
    } as DistriFnTool,

    {
      name: 'clear_map',
      description: 'Clear all markers and directions from the map',
      type: 'function',
      parameters: {
        type: 'object',
        properties: {}
      },
      handler: async (_input: object) => {
        await mapManagerRef.clearMap();
        return "Map cleared";
      }
    } as DistriFnTool
  ];
}