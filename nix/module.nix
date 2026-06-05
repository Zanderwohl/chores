{ config, lib, pkgs, ... }:

let
  cfg = config.services.chores;
in
{
  options.services.chores = {
    enable = lib.mkEnableOption "the Chores calendar/todo service";

    package = lib.mkOption {
      type = lib.types.package;
      description = "The chores package to use.";
    };

    port = lib.mkOption {
      type = lib.types.port;
      default = 3000;
      description = "Port to bind the HTTP server to.";
    };

    timezone = lib.mkOption {
      type = lib.types.str;
      default = "UTC";
      example = "America/Chicago";
      description = "Timezone for the server and displayed times.";
    };

    databasePath = lib.mkOption {
      type = lib.types.str;
      default = "/var/lib/chores/chores.db";
      description = "Path to the SQLite database file.";
    };

    dataDir = lib.mkOption {
      type = lib.types.str;
      default = "/var/lib/chores";
      description = "Working directory for the service (stores db, photos, etc).";
    };

    touchMode = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Enable touch-friendly mode with larger buttons.";
    };

    openFirewall = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Whether to open the port in the firewall.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.services.chores = {
      description = "Chores calendar/todo service";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ];

      serviceConfig = {
        Type = "simple";
        User = "chores";
        Group = "chores";
        StateDirectory = "chores";
        WorkingDirectory = cfg.dataDir;
        ExecStartPre = let
          setup = pkgs.writeShellScript "chores-setup" ''
            ln -sfn "${cfg.package}/share/chores/static" static
            ln -sfn "${cfg.package}/share/chores/migrations" migrations
            mkdir -p photos thumbnails logs
          '';
        in "${setup}";
        ExecStart = lib.concatStringsSep " " ([
          "${cfg.package}/bin/chores-unwrapped"
          "--port" (toString cfg.port)
          "--tz" cfg.timezone
        ] ++ lib.optionals cfg.touchMode [ "--touch" ]);
        Restart = "on-failure";
        RestartSec = 5;

        # Hardening
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ cfg.dataDir ];
        PrivateTmp = true;
        PrivateDevices = true;
        ProtectKernelTunables = true;
        ProtectControlGroups = true;
      };

      environment = {
        TZ = cfg.timezone;
        DATABASE_URL = cfg.databasePath;
        PORT = toString cfg.port;
      } // lib.optionalAttrs cfg.touchMode { TOUCH = "1"; };
    };

    networking.firewall.allowedTCPPorts = lib.mkIf cfg.openFirewall [ cfg.port ];
  };
}
