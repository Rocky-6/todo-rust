use anyhow::Ok;
use axum::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use thiserror::Error;
use validator::Validate;

#[derive(Debug, Error)]
enum RepositoryError {
    #[error("Unexpected Error: [{0}]")]
    Unexpected(String),
    #[error("NotFound, id is {0}")]
    NotFound(i32),
}

#[async_trait]
pub trait TodoRepository: Clone + std::marker::Send + std::marker::Sync + 'static {
    async fn create(&self, payload: CreateTodo) -> anyhow::Result<Todo>;
    async fn find(&self, id: i32) -> anyhow::Result<Todo>;
    async fn all(&self) -> anyhow::Result<Vec<Todo>>;
    async fn update(&self, id: i32, payload: UpdateTodo) -> anyhow::Result<Todo>;
    async fn delete(&self, id: i32) -> anyhow::Result<()>;
}

#[derive(Debug, Clone)]
pub struct TodoRepositoryForDB {
    pool: PgPool,
}

impl TodoRepositoryForDB {
    pub fn new(pool: PgPool) -> Self {
        TodoRepositoryForDB { pool }
    }
}

#[async_trait]
impl TodoRepository for TodoRepositoryForDB {
    async fn create(&self, payload: CreateTodo) -> anyhow::Result<Todo> {
        let todo = sqlx::query_as::<_, Todo>(
            r#"
                insert into todos (text, completed)
                values ($1, false)
                returning *
                "#,
        )
        .bind(payload.text.clone())
        .fetch_one(&self.pool)
        .await?;

        Ok(todo)
    }
    async fn find(&self, id: i32) -> anyhow::Result<Todo> {
        let todo = sqlx::query_as::<_, Todo>(
            r#"
                select * from todos where id=$1
            "#,
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => RepositoryError::NotFound(id),
            _ => RepositoryError::Unexpected(e.to_string()),
        })?;

        Ok(todo)
    }
    async fn all(&self) -> anyhow::Result<Vec<Todo>> {
        let todo = sqlx::query_as::<_, Todo>(
            r#"
                select * from todos
                order by id desc;
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(todo)
    }
    async fn update(&self, id: i32, payload: UpdateTodo) -> anyhow::Result<Todo> {
        let old_todo = self.find(id).await?;
        let todo = sqlx::query_as::<_, Todo>(
            r#"
                update todos set text=$1, completed=$2
                where id=$3
                returning *   
            "#,
        )
        .bind(payload.text.unwrap_or(old_todo.text))
        .bind(payload.completed.unwrap_or(old_todo.completed))
        .bind(id)
        .fetch_one(&self.pool)
        .await?;

        Ok(todo)
    }
    async fn delete(&self, id: i32) -> anyhow::Result<()> {
        sqlx::query(
            r#"
                delete from todos where id=$1
            "#,
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => RepositoryError::NotFound(id),
            _ => RepositoryError::Unexpected(e.to_string()),
        })?;

        Ok(())
    }
}
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, FromRow)]
pub struct Todo {
    id: i32,
    text: String,
    completed: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Validate)]
pub struct CreateTodo {
    #[validate(length(min = 1, message = "Cannot be empty"))]
    #[validate(length(max = 100, message = "Over text length"))]
    text: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Validate)]
pub struct UpdateTodo {
    #[validate(length(min = 1, message = "Cannot be empty"))]
    #[validate(length(max = 100, message = "Over text length"))]
    text: Option<String>,
    completed: Option<bool>,
}
#[cfg(test)]
pub mod test_utils {
    use anyhow::Context;
    use axum::async_trait;
    use std::{
        collections::HashMap,
        sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
    };

    use super::*;
    impl Todo {
        pub fn new(id: i32, text: String) -> Self {
            Self {
                id,
                text,
                completed: false,
            }
        }
    }

    impl CreateTodo {
        pub fn new(text: String) -> Self {
            Self { text }
        }
    }

    type TodoDatas = HashMap<i32, Todo>;

    #[derive(Debug, Clone)]
    pub struct TodoRepositoryForMemory {
        store: Arc<RwLock<TodoDatas>>,
    }

    impl TodoRepositoryForMemory {
        pub fn new() -> Self {
            TodoRepositoryForMemory {
                store: Arc::default(),
            }
        }
        fn write_store_ref(&self) -> RwLockWriteGuard<TodoDatas> {
            self.store.write().unwrap()
        }

        fn read_store_ref(&self) -> RwLockReadGuard<TodoDatas> {
            self.store.read().unwrap()
        }
    }

    #[async_trait]
    impl TodoRepository for TodoRepositoryForMemory {
        async fn create(&self, payload: CreateTodo) -> anyhow::Result<Todo> {
            let mut store = self.write_store_ref();
            let id = (store.len() + 1) as i32;
            let todo = Todo::new(id, payload.text.clone());
            store.insert(id, todo.clone());
            Ok(todo)
        }
        async fn find(&self, id: i32) -> anyhow::Result<Todo> {
            let store = self.read_store_ref();
            let todo = store
                .get(&id)
                .map(|todo| todo.clone())
                .ok_or(RepositoryError::NotFound(id))?;
            Ok(todo)
        }
        async fn all(&self) -> anyhow::Result<Vec<Todo>> {
            let store = self.read_store_ref();
            Ok(Vec::from_iter(store.values().map(|todo| todo.clone())))
        }
        async fn update(&self, id: i32, payload: UpdateTodo) -> anyhow::Result<Todo> {
            let mut store = self.write_store_ref();
            let todo = store.get(&id).context(RepositoryError::NotFound(id))?;
            let text = payload.text.unwrap_or(todo.text.clone());
            let completed = payload.completed.unwrap_or(todo.completed);
            let todo = Todo {
                id,
                text,
                completed,
            };
            store.insert(id, todo.clone());
            Ok(todo)
        }
        async fn delete(&self, id: i32) -> anyhow::Result<()> {
            let mut store = self.write_store_ref();
            store.remove(&id).ok_or(RepositoryError::NotFound(id))?;
            Ok(())
        }
    }

    #[cfg(test)]
    mod test {
        use super::*;
        use dotenv::dotenv;
        use sqlx::PgPool;
        use std::env;

        #[tokio::test]
        async fn todo_crud_scenario() {
            dotenv().ok();
            let user = env::var("POSTGRES_USER").unwrap_or("postgres".to_string());
            let password = env::var("POSTGRES_PASSWORD").unwrap_or("postgres".to_string());
            let db = env::var("POSTGRES_DB").unwrap_or("postgres".to_string());
            let host = env::var("POSTGRES_HOSTNAME").unwrap_or("localhost".to_string());
            let port = env::var("POSTGRES_PORT").unwrap_or("5432".to_string());
            let database_url = &format!(
                "postgresql://{}:{}@{}:{}/{}",
                user, password, host, port, db
            );
            let pool = PgPool::connect(database_url)
                .await
                .expect(&format!("fail connect database, url is [{}]", database_url));
            let repository = TodoRepositoryForDB::new(pool.clone());
            let text = "todo text";

            let created = repository
                .create(CreateTodo::new(text.to_string()))
                .await
                .expect("[create] returned error");
            assert_eq!(created.text, text);
            assert!(!created.completed);

            let todo = repository
                .find(created.id)
                .await
                .expect("[find] returned error");
            assert_eq!(created, todo);

            let todos = repository.all().await.expect("[all] returned error");
            let todo = todos.first().unwrap();
            assert_eq!(created, *todo);

            let updated_text = "[crud_scenario] updated text";
            let todo = repository
                .update(
                    todo.id,
                    UpdateTodo {
                        text: Some(updated_text.to_string()),
                        completed: Some(true),
                    },
                )
                .await
                .expect("[update] returned error");
            assert_eq!(created.id, todo.id);
            assert_eq!(todo.text, updated_text);

            let _ = repository
                .delete(todo.id)
                .await
                .expect("[delete] returned error");
            let res = repository.find(created.id).await;
            assert!(res.is_err());

            let todo_rows = sqlx::query(
                r#"
                    select * from todos where id = $1
                "#,
            )
            .bind(todo.id)
            .fetch_all(&pool)
            .await
            .expect("[delete] todo_labels fetch error");
            assert!(todo_rows.len() == 0);
        }
    }
}
