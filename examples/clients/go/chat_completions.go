package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"strings"
	"time"
)

func main() {
	baseURL := os.Getenv("DITTO_BASE_URL")
	if baseURL == "" {
		baseURL = "http://127.0.0.1:8080"
	}
	baseURL = strings.TrimRight(baseURL, "/")
	token := os.Getenv("DITTO_VK_TOKEN")
	if token == "" {
		fmt.Fprintln(os.Stderr, "missing DITTO_VK_TOKEN")
		os.Exit(1)
	}

	payload := map[string]any{
		"model":  "gpt-4o-mini",
		"stream": false,
		"messages": []map[string]any{
			{"role": "user", "content": "Say hello in one sentence."},
		},
	}

	body, err := json.Marshal(payload)
	if err != nil {
		panic(err)
	}

	req, err := http.NewRequest("POST", baseURL+"/v1/chat/completions", bytes.NewReader(body))
	if err != nil {
		panic(err)
	}
	req.Header.Set("content-type", "application/json")
	req.Header.Set("authorization", "Bearer "+token)
	req.Header.Set("x-request-id", fmt.Sprintf("go-%d", time.Now().UnixMilli()))

	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		panic(err)
	}
	defer resp.Body.Close()

	respBody, _ := io.ReadAll(resp.Body)
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		fmt.Fprintf(os.Stderr, "HTTP %d: %s\n", resp.StatusCode, string(respBody))
		os.Exit(1)
	}
	fmt.Println(string(respBody))
}
