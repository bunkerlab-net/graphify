// TypeScript client for the server
interface User {
  id: number;
  name: string;
  email: string;
}

interface ApiResponse<T> {
  data?: T;
  error?: string;
}

class ApiClient {
  private baseUrl: string;

  constructor(baseUrl: string) {
    this.baseUrl = baseUrl;
  }

  async getHealth(): Promise<{ status: string }> {
    const response = await fetch(`${this.baseUrl}/health`);
    return response.json();
  }

  async getUsers(): Promise<ApiResponse<User[]>> {
    const response = await fetch(`${this.baseUrl}/users`);
    if (!response.ok) {
      return { error: `HTTP ${response.status}` };
    }
    const data = await response.json();
    return { data: data.users };
  }

  async createUser(name: string, email: string): Promise<ApiResponse<User>> {
    const response = await fetch(`${this.baseUrl}/users`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name, email }),
    });
    if (!response.ok) {
      return { error: `HTTP ${response.status}` };
    }
    return { data: await response.json() };
  }
}

function formatUser(user: User): string {
  return `${user.name} <${user.email}> (id=${user.id})`;
}

export { ApiClient, formatUser };
export type { User, ApiResponse };
