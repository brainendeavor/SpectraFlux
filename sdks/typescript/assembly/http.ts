// AssemblyScript SDK: HTTP Domain Models & Helpers

export class RouteMeta {
  method: string;
  path: string;
  description: string;

  constructor(method: string, path: string, description: string = "") {
    this.method = method;
    this.path = path;
    this.description = description;
  }

  static get(path: string, description: string = ""): RouteMeta {
    return new RouteMeta("GET", path, description);
  }

  static post(path: string, description: string = ""): RouteMeta {
    return new RouteMeta("POST", path, description);
  }

  static put(path: string, description: string = ""): RouteMeta {
    return new RouteMeta("PUT", path, description);
  }

  static delete(path: string, description: string = ""): RouteMeta {
    return new RouteMeta("DELETE", path, description);
  }

  toJson(): string {
    return '{"method":"' + this.method + '","path":"' + this.path + '","description":"' + this.description + '"}';
  }
}

export class HttpRequest {
  path: string;
  method: string;
  headers: Array<string[]>;
  body: string;

  constructor(path: string = "/", method: string = "GET", body: string = "", headers: Array<string[]> = new Array<string[]>()) {
    this.path = path;
    this.method = method;
    this.body = body;
    this.headers = headers;
  }

  getHeader(name: string): string | null {
    const target = name.toLowerCase();
    for (let i = 0; i < this.headers.length; i++) {
      const pair = this.headers[i];
      if (pair.length >= 2 && pair[0].toLowerCase() == target) {
        return pair[1];
      }
    }
    return null;
  }

  static fromJson(jsonStr: string): HttpRequest {
    let path = "/";
    let method = "GET";
    let body = "";

    // Extract path
    const pathIdx = jsonStr.indexOf('"path":');
    if (pathIdx != -1) {
      const start = jsonStr.indexOf('"', pathIdx + 7);
      if (start != -1) {
        const end = jsonStr.indexOf('"', start + 1);
        if (end != -1) {
          path = jsonStr.substring(start + 1, end);
        }
      }
    }

    // Extract method
    const methodIdx = jsonStr.indexOf('"method":');
    if (methodIdx != -1) {
      const start = jsonStr.indexOf('"', methodIdx + 9);
      if (start != -1) {
        const end = jsonStr.indexOf('"', start + 1);
        if (end != -1) {
          method = jsonStr.substring(start + 1, end);
        }
      }
    }

    // Extract body
    const bodyIdx = jsonStr.indexOf('"body":');
    if (bodyIdx != -1) {
      const rest = jsonStr.substring(bodyIdx + 7).trimStart();
      if (rest.startsWith('"')) {
        const end = rest.indexOf('"', 1);
        if (end != -1) {
          body = rest.substring(1, end);
        }
      } else {
        body = rest;
      }
    }

    return new HttpRequest(path, method, body);
  }
}

export class HttpResponse {
  status: u16;
  headers: Array<string[]>;
  body: string;

  constructor(status: u16 = 200, body: string = "", headers: Array<string[]> = new Array<string[]>()) {
    this.status = status;
    this.body = body;
    this.headers = headers;
  }

  static ok(body: string = "ok"): HttpResponse {
    return new HttpResponse(200, body);
  }

  static json(bodyJson: string, status: u16 = 200): HttpResponse {
    const headers = new Array<string[]>();
    headers.push(["content-type", "application/json"]);
    return new HttpResponse(status, bodyJson, headers);
  }

  static notFound(message: string = "Endpoint not found"): HttpResponse {
    const headers = new Array<string[]>();
    headers.push(["content-type", "application/json"]);
    return new HttpResponse(404, '{"error":"NOT_FOUND","message":"' + message + '"}', headers);
  }

  static error(message: string, status: u16 = 500): HttpResponse {
    const headers = new Array<string[]>();
    headers.push(["content-type", "application/json"]);
    return new HttpResponse(status, '{"error":"INTERNAL_ERROR","message":"' + message + '"}', headers);
  }

  toJson(): string {
    let out = '{"status":' + this.status.toString() + ',"headers":[';
    for (let i = 0; i < this.headers.length; i++) {
      if (i > 0) out += ",";
      const h = this.headers[i];
      out += '["' + h[0] + '","' + h[1] + '"]';
    }
    out += '],"body":"' + escapeJson(this.body) + '"}';
    return out;
  }
}

function escapeJson(s: string): string {
  let out = "";
  for (let i = 0; i < s.length; i++) {
    const code = s.charCodeAt(i);
    if (code == 34) { // "
      out += '\\"';
    } else if (code == 92) { // \
      out += '\\\\';
    } else if (code == 10) { // \n
      out += '\\n';
    } else if (code == 13) { // \r
      out += '\\r';
    } else if (code == 9) { // \t
      out += '\\t';
    } else {
      out += s.charAt(i);
    }
  }
  return out;
}
