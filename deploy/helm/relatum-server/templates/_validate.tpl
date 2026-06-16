{{- /*
Interchangeability guard. relatum-server pods are only interchangeable when all
state is external (postgres + redis): the in-memory backends keep users, reports
and sessions in-process, so multiple replicas would diverge. Refuse that combo
rather than silently serve inconsistent data. Included from the Deployment.
*/ -}}
{{- define "relatum-server.validateValues" -}}
{{- $replicas := .Values.replicaCount | int -}}
{{- if .Values.autoscaling.enabled -}}
{{- $replicas = max $replicas (.Values.autoscaling.maxReplicas | int) | int -}}
{{- end -}}
{{- if gt $replicas 1 -}}
{{- if or (eq .Values.config.data.backend "memory") (eq .Values.config.sessions.backend "memory") -}}
{{- fail "the in-memory backends are not replica-safe: set config.data.backend=postgres and config.sessions.backend=redis, or keep replicaCount=1 with autoscaling disabled" -}}
{{- end -}}
{{- end -}}
{{- if eq .Values.config.sso.backend "oidc" -}}
{{- if not .Values.config.sso.oidc.userinfoUrl -}}
{{- fail "config.sso.backend is \"oidc\" but config.sso.oidc.userinfoUrl is empty: set it to the provider's userinfo endpoint" -}}
{{- end -}}
{{- if not .Values.config.sso.oidc.authorizeUrl -}}
{{- fail "config.sso.backend is \"oidc\" but config.sso.oidc.authorizeUrl is empty: set it to the provider's authorization endpoint" -}}
{{- end -}}
{{- if not .Values.config.sso.oidc.tokenUrl -}}
{{- fail "config.sso.backend is \"oidc\" but config.sso.oidc.tokenUrl is empty: set it to the provider's token endpoint" -}}
{{- end -}}
{{- if not .Values.config.sso.oidc.clientId -}}
{{- fail "config.sso.backend is \"oidc\" but config.sso.oidc.clientId is empty: set the OAuth client id" -}}
{{- end -}}
{{- if not .Values.config.sso.oidc.publicUrl -}}
{{- fail "config.sso.backend is \"oidc\" but config.sso.oidc.publicUrl is empty: set the server's externally reachable base URL" -}}
{{- end -}}
{{- if and (not .Values.config.sso.oidc.clientSecret) (not .Values.config.sso.oidc.clientSecretExistingSecret) -}}
{{- fail "config.sso.backend is \"oidc\" but no client secret is configured: set config.sso.oidc.clientSecret or config.sso.oidc.clientSecretExistingSecret" -}}
{{- end -}}
{{- end -}}
{{- if .Values.httproute.enabled -}}
{{- if not .Values.httproute.parentRefs -}}
{{- fail "httproute.enabled is true but httproute.parentRefs is empty: set at least one parentRef naming the Gateway to attach to (e.g. httproute.parentRefs[0].name=external-gateway)" -}}
{{- end -}}
{{- end -}}
{{- end -}}
