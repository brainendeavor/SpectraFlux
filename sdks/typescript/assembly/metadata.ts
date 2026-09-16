// AssemblyScript SDK: Fluxcell Metadata Model

export class FluxcellMetadata {
  version: string;
  gitHash: string;
  buildTime: string;
  description: string;

  constructor(version: string, description: string = "", gitHash: string = "unknown", buildTime: string = "") {
    this.version = version;
    this.description = description;
    this.gitHash = gitHash;
    this.buildTime = buildTime;
  }

  toJson(): string {
    return '{"version":"' + this.version + '","git_hash":"' + this.gitHash + '","build_time":"' + this.buildTime + '","description":"' + this.description + '"}';
  }
}
