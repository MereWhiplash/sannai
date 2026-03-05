package main

import (
	"log"
	"os"

	"github.com/gin-gonic/gin"
)

func main() {
	port := os.Getenv("PORT")
	if port == "" {
		port = "8080"
	}

	r := gin.Default()

	// Health check
	r.GET("/healthz", func(c *gin.Context) {
		c.JSON(200, gin.H{"status": "ok"})
	})

	// API v1 routes
	v1 := r.Group("/api/v1")
	{
		// Session sync (from local agents)
		v1.POST("/sessions", notImplemented)
		v1.POST("/sessions/:id/events", notImplemented)
		v1.POST("/commits", notImplemented)

		// Dashboard API
		v1.GET("/sessions", notImplemented)
		v1.GET("/sessions/:id", notImplemented)
		v1.GET("/sessions/:id/timeline", notImplemented)

		// PR integration
		v1.GET("/prs/:owner/:repo/:number", notImplemented)
		v1.POST("/webhooks/github", notImplemented)

		// Auth
		v1.GET("/auth/sso/:provider", notImplemented)
		v1.POST("/auth/sso/callback", notImplemented)
		v1.POST("/auth/token", notImplemented)

		// Admin
		v1.GET("/admin/users", notImplemented)
		v1.GET("/admin/analytics", notImplemented)
		v1.POST("/admin/export", notImplemented)
	}

	log.Printf("Starting sannai API on :%s", port)
	if err := r.Run(":" + port); err != nil {
		log.Fatalf("Failed to start server: %v", err)
	}
}

func notImplemented(c *gin.Context) {
	c.JSON(501, gin.H{"error": "not implemented"})
}
