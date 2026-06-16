{{- /*
Config guard. The frontend is stateless — the SSO login nonce rides in a browser
cookie, not in process memory — so any replica count is safe and there is no
interchangeability check to make. It only has to know where the API is and what
its own externally-reachable URL is. Included from the Deployment.
*/ -}}
{{- define "relatum-web.validateValues" -}}
{{- if not .Values.config.apiUrl -}}
{{- fail "config.apiUrl is empty: set it to the relatum-server base URL the frontend should call (e.g. http://relatum-server)" -}}
{{- end -}}
{{- if not .Values.config.publicUrl -}}
{{- fail "config.publicUrl is empty: set the frontend's externally-reachable base URL — it builds the SSO redirect_uri and decides the Secure cookie flag" -}}
{{- end -}}
{{- if .Values.httproute.enabled -}}
{{- if not .Values.httproute.parentRefs -}}
{{- fail "httproute.enabled is true but httproute.parentRefs is empty: set at least one parentRef naming the Gateway to attach to (e.g. httproute.parentRefs[0].name=external-gateway)" -}}
{{- end -}}
{{- end -}}
{{- end -}}
