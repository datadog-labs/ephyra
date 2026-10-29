from pydantic_settings import BaseSettings, SettingsConfigDict


class Settings(BaseSettings):
    model_config = SettingsConfigDict(extra="allow")

    name: str


s = Settings(name="app", debug=True)
print(s)
