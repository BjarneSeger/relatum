{{/*
Expand the name of the chart.
*/}}
{{- define "relatum-server.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Fully qualified app name. Truncated to 63 chars for DNS label limits.
*/}}
{{- define "relatum-server.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{- define "relatum-server.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "relatum-server.labels" -}}
helm.sh/chart: {{ include "relatum-server.chart" . }}
{{ include "relatum-server.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{- define "relatum-server.selectorLabels" -}}
app.kubernetes.io/name: {{ include "relatum-server.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "relatum-server.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "relatum-server.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
Name of the bundled Postgres StatefulSet / Service / managed credentials.
*/}}
{{- define "relatum-server.postgresName" -}}
{{- printf "%s-postgresql" (include "relatum-server.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Service name produced by the valkey subchart (its default fullname).
*/}}
{{- define "relatum-server.valkeyName" -}}
{{- printf "%s-valkey" .Release.Name | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Whether the chart manages (templates) the database URL secret itself, vs the
user pointing at their own existing secret.
*/}}
{{- define "relatum-server.manageDbSecret" -}}
{{- and (eq .Values.config.data.backend "postgres") (not .Values.database.existingSecret) -}}
{{- end }}

{{- define "relatum-server.dbSecretName" -}}
{{- if .Values.database.existingSecret }}
{{- .Values.database.existingSecret }}
{{- else }}
{{- printf "%s-db" (include "relatum-server.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}

{{- define "relatum-server.dbSecretKey" -}}
{{- if .Values.database.existingSecret }}
{{- .Values.database.existingSecretKey }}
{{- else }}
{{- "url" }}
{{- end }}
{{- end }}

{{/*
Same idea for the Valkey/session URL secret.
*/}}
{{- define "relatum-server.manageSessionsSecret" -}}
{{- and (eq .Values.config.sessions.backend "redis") (not .Values.sessions.existingSecret) -}}
{{- end }}

{{- define "relatum-server.sessionsSecretName" -}}
{{- if .Values.sessions.existingSecret }}
{{- .Values.sessions.existingSecret }}
{{- else }}
{{- printf "%s-sessions" (include "relatum-server.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}

{{- define "relatum-server.sessionsSecretKey" -}}
{{- if .Values.sessions.existingSecret }}
{{- .Values.sessions.existingSecretKey }}
{{- else }}
{{- "url" }}
{{- end }}
{{- end }}

{{/*
Same idea for the OIDC client secret (only when SSO uses the oidc backend).
*/}}
{{- define "relatum-server.manageSsoSecret" -}}
{{- and (eq .Values.config.sso.backend "oidc") (not .Values.config.sso.oidc.clientSecretExistingSecret) -}}
{{- end }}

{{- define "relatum-server.ssoSecretName" -}}
{{- if .Values.config.sso.oidc.clientSecretExistingSecret }}
{{- .Values.config.sso.oidc.clientSecretExistingSecret }}
{{- else }}
{{- printf "%s-sso" (include "relatum-server.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}

{{- define "relatum-server.ssoSecretKey" -}}
{{- if .Values.config.sso.oidc.clientSecretExistingSecret }}
{{- .Values.config.sso.oidc.clientSecretExistingSecretKey }}
{{- else }}
{{- "client-secret" }}
{{- end }}
{{- end }}

{{/*
Resolve the Postgres password: explicit value, else a value retained from a
prior install's managed secret, else freshly generated. Evaluating this more
than once per render is safe because the lookup pins it after the first install.
*/}}
{{- define "relatum-server.postgresPassword" -}}
{{- if .Values.postgresql.password -}}
{{- .Values.postgresql.password -}}
{{- else -}}
{{- $secretName := include "relatum-server.dbSecretName" . -}}
{{- $existing := lookup "v1" "Secret" .Release.Namespace $secretName -}}
{{- if and $existing $existing.data (index $existing.data "password") -}}
{{- index $existing.data "password" | b64dec -}}
{{- else -}}
{{- randAlphaNum 32 -}}
{{- end -}}
{{- end -}}
{{- end }}

{{/*
The full RELATUM_SESSIONS_URL: inline url, else the bundled (no-auth) Valkey.
(The database URL is built inline in secret.yaml so its single resolved password
is shared with the `password` key — see the note there.)
*/}}
{{- define "relatum-server.sessionsUrl" -}}
{{- if .Values.sessions.url -}}
{{- .Values.sessions.url -}}
{{- else if .Values.valkey.enabled -}}
{{- printf "redis://%s:%v" (include "relatum-server.valkeyName" .) .Values.valkey.service.port -}}
{{- else -}}
{{- fail "config.sessions.backend is \"redis\" but no Valkey source is configured: set sessions.existingSecret, sessions.url, or valkey.enabled" -}}
{{- end -}}
{{- end }}
