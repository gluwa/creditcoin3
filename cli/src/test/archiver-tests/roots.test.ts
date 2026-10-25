import axios from 'axios';

describe('GET /roots', (): void => {
    const archiverUrl = (global as any).ARCHIVER_URL;

    test('returns an error when both from & to fields are missing', async (): Promise<void> => {
        // will fail if the call below suddenly succeeds
        expect.assertions(2);

        try {
            await axios.get(`${archiverUrl}/roots`);
        } catch (error) {
            const response = (error as any).response;
            expect(response.status).toBe(400);
            expect(response.data).toEqual('Failed to deserialize query string: missing field `from`');
        }
    });

    test('returns a response', async (): Promise<void> => {
        const response = await axios.get(`${archiverUrl}/roots?from=0&to=1`);
        expect(response.status).toBe(200);

        expect(response.data.length).toBe(2);
        for (const data of response.data) {
            expect(data).toHaveProperty('block_number');
            expect(data).toHaveProperty('merkle_root');
        }
    });
});
