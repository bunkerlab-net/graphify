// Package proxy implements a simple reverse proxy.
package proxy

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"time"
)

// Config holds proxy configuration.
type Config struct {
	ListenAddr string
	TargetURL  string
	Timeout    time.Duration
}

// DefaultConfig returns a Config with sensible defaults.
func DefaultConfig() Config {
	return Config{
		ListenAddr: ":9090",
		TargetURL:  "http://localhost:8080",
		Timeout:    30 * time.Second,
	}
}

// Proxy forwards requests to a backend server.
type Proxy struct {
	config Config
	client *http.Client
	target *url.URL
}

// New creates a Proxy from cfg.
func New(cfg Config) (*Proxy, error) {
	target, err := url.Parse(cfg.TargetURL)
	if err != nil {
		return nil, fmt.Errorf("invalid target URL: %w", err)
	}
	return &Proxy{
		config: cfg,
		client: &http.Client{Timeout: cfg.Timeout},
		target: target,
	}, nil
}

// ServeHTTP implements http.Handler.
func (p *Proxy) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	upstream := *p.target
	upstream.Path = r.URL.Path
	upstream.RawQuery = r.URL.RawQuery

	req, err := http.NewRequestWithContext(r.Context(), r.Method, upstream.String(), r.Body)
	if err != nil {
		p.writeError(w, http.StatusBadGateway, err.Error())
		return
	}
	for k, vs := range r.Header {
		for _, v := range vs {
			req.Header.Add(k, v)
		}
	}

	resp, err := p.client.Do(req)
	if err != nil {
		p.writeError(w, http.StatusBadGateway, err.Error())
		return
	}
	defer resp.Body.Close()

	for k, vs := range resp.Header {
		for _, v := range vs {
			w.Header().Add(k, v)
		}
	}
	w.WriteHeader(resp.StatusCode)
	io.Copy(w, resp.Body)
}

func (p *Proxy) writeError(w http.ResponseWriter, status int, msg string) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	json.NewEncoder(w).Encode(map[string]string{"error": msg})
}

// ListenAndServe starts the proxy server.
func (p *Proxy) ListenAndServe() error {
	fmt.Printf("Proxy listening on %s → %s\n", p.config.ListenAddr, p.config.TargetURL)
	return http.ListenAndServe(p.config.ListenAddr, p)
}
